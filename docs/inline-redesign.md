# Camouflage inline redesign

Working document for the Camouflage redesign and the autopilot idle-CPU hotfix. It records what's wrong today, what we decided, and what has been done. Update the **Work log** at the bottom whenever something lands.

- Started: 2026-09-28
- Prototype: https://claude.ai/artifact/H3fMXtBtVoTfTMm74S8HJe (private; browser mock of the proposed look, with a toggle to the v2.1 look)
- Related repos: `~/camouflage` (this repo), `~/autopilot` (the harness that uses Camouflage and Ink)

## 1. Where Camouflage stands

Camouflage is a Rust/ratatui renderer that a host drives with NDJSON events. It stores every event in SQLite before drawing, so sessions can be replayed. That core idea is good and nobody else does it. The interaction layer on top of it is weak. Using it in autopilot felt "50–60% ok."

In practice it's a fixed chat-app interface driven over a pipe, not a library: 41 hard-coded event types (`crates/protocol/src/lib.rs:45`), one fixed layout, and a fixed set of widgets. The host can't set the theme, keymap, layout or rows.

### 1.1 Bugs, by severity

**Critical (you notice within seconds)**

| # | Problem | Where |
|---|---|---|
| C1 | Non-ASCII typing is dropped. The key parser only accepts bytes 0x20–0x7e. | `crates/tui/src/tty.rs:204` |
| C2 | Pasted UTF-8 becomes mojibake. Each byte is pushed as its own character ("café" → "cafÃ©"). | `crates/tui/src/tty.rs:336` |
| C3 | `T` `M` `X` `S` `?` are global shortcuts when the input is empty. "Should I…" turns mouse capture off; "The…" changes the theme. | `crates/tui/src/input.rs:142-146` |
| C4 | Full-screen mode plus mouse capture by default. The transcript disappears when you quit, and you can't select text normally. | `crates/tui/src/app.rs:2117`, `:2131` |
| C5 | The README quick start shows nothing. The default `mount()` pipes the renderer's stdout back into the SDK, so the screen output goes into the event parser. | `sdk/node/src/binding.js:234-235` |

**High**

| # | Problem | Where |
|---|---|---|
| H1 | Rows can't be changed once shown: no update, remove or collapse. Hosts pre-format diffs as ANSI themselves. | protocol |
| H2 | Markdown is a hand-written parser for inline styles only. Block formatting is guessed from the first characters of each line, and there's no syntax highlighting anywhere. | `crates/renderer/src/markdown.rs:11-15` |
| H3 | Up/Down never move within multi-line input. Shift+Enter only works on kitty or xterm modifyOtherKeys terminals. No undo or yank. The cursor moves by character, not by what the user sees as one character. | `input.rs:235`, `tty.rs:249`, `tty.rs:302` |
| H4 | Forms: Tab doesn't move between fields (the code checks `Char('\t')`, but the parser sends `Key::Tab`), and paste is silently ignored. | `app.rs:810`, `app.rs:863` |
| H5 | The SDK's promise helpers never settle if the renderer exits. | `binding.js:364` and following |
| H6 | The TypeScript types are behind the protocol (no `TodoListUpdate`, `ShowToast`, `Splash` or `TranscriptCleared`), and `send()` accepts any string. autopilot deleted `TranscriptCleared` calls thinking it didn't exist. | `sdk/node/src/index.d.ts` |
| H7 | The panic handler leaves mouse reporting and bracketed paste on, and writes `crash-*.ndjson` into the user's current directory. | `app.rs:2153-2178` |
| H8 | In `renderToTerminal` mode, a `console.log` from the host corrupts the screen. | SDK |
| H9 | Regression: `66bc2f4` removed the guard from `924eb7e`, so pressing Up while typing overwrites your draft. | `app.rs:752` |

**Medium (structural)**

- `app.rs` `run()` is about a 1,500-line function with ~40 local mutable variables. Key routing is stacked `if` checks with no focus stack. `Event { … }` construction is copy-pasted a dozen times. `draw::render` takes 22 positional arguments (`draw.rs:161`).
- The hand-written key parser works around a macOS kqueue problem (`tty.rs:1-13`). crossterm's `use-dev-tty` feature exists for exactly that.
- Search only covers rows in memory, not SQLite (`app.rs:2032`).
- Every session is saved by default, unencrypted and with no retention limit, to `~/.camouflage/sessions.db` (`main.rs:83`).
- No Windows support (`/dev/tty`, `libc`, `from_raw_fd`).
- Unknown event types are skipped silently (`ac39f2d`). autopilot's 104 `ShowToast` calls were dropped for weeks with no error (`030870c`). There's no version check between host and renderer.

### 1.2 What the commit history shows

168 commits between 2026-05-16 and 2026-06-03.

- **The ambitious features came first.** v0.1 through v0.6 shipped on day one: replay, inspector, filters, search, bookmarks, a browser viewer, websocket broadcast, a validator, export, golden tests, themes, metrics. Daily use started on May 17. After that, most commits are "TUI bug batch" fixes to basic interaction: `q` quits the app, Esc doesn't abort, Ctrl+C doesn't really exit, the mouse wheel walks prompt history, drafts get overwritten.
- **Key handling was fixed one symptom at a time** (`fbcd478`, `1e2c50c`, `f32dae4`, `7fde7b4`, `b4febc9`, `620eef2`, `47abe99`, `72b3c5d`). The root cause is the missing focus model.
- **The scroll math** (`1d01e94`: counting wrapped lines, freezing the scroll anchor, an unstable ratatui API) is the cost of running a custom full-screen viewport.
- **API churn:** Toast was added, removed in a breaking 2.0.0 (`da50df1`), then added back as `ShowToast` the next day.
- **The visual polish commits** (`5e992c5`, `dbd9d25`, `da80961`, `9a6af0f`) pushed toward a chat-log look. We think that's the wrong target (see §2).

### 1.3 What's worth keeping

The event log, persisting before rendering, replay, the protocol any language can drive, bracketed paste, double Ctrl+C to quit, Esc cancelling from anywhere, and the existing tests for the key parser and protocol.

## 2. Visual direction

Goal: feel like a terminal program that prints its output (Claude Code), not an app drawn inside the terminal (v2.1, opencode).

What looks wrong in v2.1 (from the `tests/visual/out/` screenshots):

- Debug output shows in the product: `session=…` in the header (`draw.rs:317`), a `· session started` row, an input box titled `input`, `idle` in the status bar.
- Full-screen layout: content at the top, the input at the bottom, a large empty gap between them.
- It reads like an IRC log: `You:` / `autopilot:` labels plus a rule and padding after every message, about 4 lines of framing per message.
- Too many strong fixed-RGB colors: blue, purple, cyan, yellow, orange.
- 17 square-cornered `Borders::ALL` boxes in `draw.rs`, including heavy boxed toasts.

The proposed design, as built in the prototype:

| Element | Rule |
|---|---|
| Color | Use the terminal's own color palette plus **one** accent color chosen by the host. Red and green only for errors, success and diffs. Everything secondary is dim. |
| User prompt | A faint tinted band across the full width, with `›` in dim. No speaker label. |
| Assistant text | No label, indented 2 columns. |
| Tools | The status icon sits in column 0 (spinner / `✓` green / `✗` red / `⊘` declined / `■` stopped), then the **name** in bold and the arguments. Underneath: `└ summary` in dim. Diffs appear inline with line numbers and a light red/green background. Long output collapses behind "ctrl+o to expand." |
| Plan | Pinned above the input while a turn runs: `◆ Plan · 1 of 4 done`, then `✓` done (struck through), `›` current (bold), `○` pending. Printed into the transcript only if the turn stops before it finishes. |
| Spinner line | `⠹ Verb… (12s · ↓ 1.2k tokens · esc to interrupt)`, with a shimmer on the verb. |
| Input | One rounded box. The border color shows the mode: dim for default, yellow for accept edits, cyan for plan. |
| Footer | Dim text under the input: mode or `? for shortcuts` on the left; model, tokens and cost on the right. Short messages ("Model set to…") also appear here, replacing toasts. |
| Pickers | `/` and `@` lists open under the input. File matches show the matched letters in bold. |
| Prompts | Permission and selection prompts replace the input box: a rounded box with an accent border and numbered options. |
| Markdown | Headings, lists, tables (drawn with box-drawing characters, no outer border), quotes, and code with a dim `│` gutter and syntax highlighting. |
| Welcome | Three dim lines: name and version with the model, the working directory and branch, and hints. No session id. |
| Exit | The transcript stays and a resume command is printed. |

## 3. Architecture: inline rendering

Finished blocks are printed **once** into the terminal's normal scrollback and never redrawn. Only a small live area at the bottom is redrawn: the in-progress block, the plan, the spinner, the input and the footer. SQLite stays the source of truth for replay and resume.

What this gives you:
- You can scroll with the terminal's own scrollback while the agent works.
- Memory stays flat, because printed blocks can be dropped from the app.
- Text selection works normally.
- The transcript is still there after you quit.
- Drawing cost depends on the size of the live area, not the length of the session.

Rules that make it work:
1. **The live area must always be shorter than the screen.** A streaming answer is printed paragraph by paragraph and only the unfinished part stays live. Running tool output shows only its last few lines. The plan panel has a maximum height. If the live area ever grows past the screen, it's back to clearing the terminal, which is exactly how Ink breaks (`ink/build/ink.js:705`).
2. **Never clear the terminal**, except on an explicit `/clear`.
3. **Printed blocks are final.** Ctrl+O "expand" opens a separate full-screen transcript view built from SQLite. The current viewport, paging and search code gets reused there.
4. **No full-width boxes or rules in printed output.** The terminal re-wraps old text on resize, but full-width lines break badly. Boxes belong only in the live area, which is redrawn at the new width anyway.

Trade-off: the terminal's scrollback limit caps how far back you can scroll. Older history stays reachable through SQLite, the transcript view and `--resume`.

Protocol changes this needs: stable host-chosen row ids; `RowUpdated`, `RowRemoved`, `RowCollapsed` (for the live area); a generic block made of styled spans; host-set theme, accent and keymap; a version check at startup; a warning for unknown events.

## 4. Performance: idle must mean zero work

### 4.1 What happened in autopilot (2026-09-28)

Two idle autopilot sessions, each about 27 hours old, were running at about 127% CPU with 1.3 GB and 2.6 GB of memory. A fresh session sat at 0%. A 3-second `sample` of the hot process showed:
- constant `write` calls: it was redrawing the terminal the whole time,
- constant Unicode text measurement (the segmentation calls Ink uses to size strings), across the whole transcript,
- most of the remaining time in V8 garbage collection, cleaning up the short-lived strings those redraws create.

Why:
- `ChatView` (`src/ui/chat.tsx:58`) renders every event in the live React tree, and nothing uses `<Static>`. Each frame lays out the entire session.
- Something keeps animating while idle, so the frames never stop. The elapsed-time timers (`status.tsx:47`, `tool-view.tsx:45`) stop correctly, and task rows are cleared at the end of every turn. That leaves the spinners, each running its own 80–130 ms timer (`ink-spinner`). Candidates that can outlive a turn:
  - the cost spinner while spend is being confirmed (`status.tsx:136`)
  - worker spinners (`worker-list.tsx:70`, `:99`, `:195`)
  - the streaming spinner (`chat.tsx:181`) on an assistant message that turn cleanup doesn't reset
- Which one was actually running is **not yet confirmed**.

Camouflage v2.1 has the same class of bug:
- `spinner_alive` (`crates/tui/src/app.rs:1891`) redraws the whole screen at 60 fps as long as any tool hasn't received its finished event, the phase is `thinking`, or a todo is in progress. A host that forgets one event leaves it spinning forever.
- The frame timer wakes 60 times a second and the keyboard reader 20 times a second, even when fully idle.
- Every redraw walks every row (`draw.rs:433`).

### 4.2 Requirements (for both the Camouflage rewrite and the autopilot hotfix)

1. **Printed output is the default.** Finished blocks are written once and dropped from the live tree.
2. **Idle means zero work:** no timers, no drawing, no allocation. One shared animation clock runs only while something visible is animating, and it stops completely otherwise. No component starts its own timer.
3. **The renderer settles everything when a turn ends.** Spinners stop even if the host never sends the finish events.
4. **Drawing cost depends on the live area, not the history.** Anything scrollable is virtualized.
5. **Track what changed.** Redraw only what changed, and skip frames that would be identical to the last one. Cap the frame rate.
6. **Keep the hot path low-allocation.** Cache text widths and wrapping.
7. **Slow down or pause when the terminal window loses focus** (terminal focus events, `CSI ?1004h`).
8. **CI soak test:** draw 10k, 50k and 100k lines, then go idle. Idle CPU must be about 0 and the same at every size, memory must level off, and there must be zero wakeups with no animation running. With one spinner visible, the time per frame must not change between 1k and 100k lines.

## 5. Related bug: `usage.json` gets corrupted

`~/.local/share/kimiflare/usage.json` was found with extra bytes after the end of the JSON.
- `withLock` (`src/usage-tracker.ts:178`) only prevents overlapping writes within one process, and `saveLog` overwrites the file in place, so two autopilot processes can interleave their writes.
- `loadLog` quietly returns an empty log when it can't parse the file, so the next save **overwrites the whole cost history** with that empty log.

Fix: write to a temp file and rename it into place, lock across processes, and never save over a file that failed to parse (move the broken file aside and warn).

## 6. Adoption: how a harness puts Camouflage on

A renderer nobody can adopt is worthless. **autopilot is the first customer, and every design decision has to pass one test: does it make autopilot's Camouflage mode as good as or better than its Ink mode, with less glue code?** Last time, autopilot's integration (`src/ui-mode.ts`, 3,300+ lines) turned into a long list of ports and workarounds (see `~/autopilot/CAMOUFLAGE_MIGRATION.md`): theme changes that only applied "on the next Ink session", diffs pre-formatted as ANSI on the host side, a `setInterval` pushing elapsed-time updates, a mouse-click handler for an event the renderer never sent, and `TranscriptCleared` calls deleted because the types said it didn't exist.

**Current state (2026-09-29):** autopilot has **no** Camouflage mode on `main`. It was disabled on 2026-06-30 (autopilot #597, "temporarily disable Camouflage UI access and force Ink"), and the 3,609-line `src/ui-mode.ts` was deleted as dead code on 2026-09-25 (autopilot #639). The new integration gets written from scratch against the new renderer. That's an opportunity: it can be designed small from the start instead of carrying the old workarounds.

Adoption requirements:
1. **Integration size is a metric.** Track the lines of host glue code autopilot needs. The target is under ~800 for full parity with the Ink UI. When glue code grows, that points at a missing renderer feature, not a host problem.
2. **Real components, not generic ones.** Ship first-class support for what every agent harness has: turns, streaming text, tool calls with a status and a result, diffs, permission prompts, plans/todos, a status/footer, pickers, queued messages, sub-agent/worker lists. The host sends meaning ("this tool finished with this diff"); the renderer owns how it looks.
3. **An escape hatch for the rest:** a generic block of styled spans with a stable id the host can update, so a harness never gets stuck waiting on a new event type (autopilot's QR code, cost report, hooks dashboard).
4. **Host-owned identity:** name, accent color, welcome lines, keymap overrides, slash commands with argument hints, labels. No Camouflage branding.
5. **Timers belong to the renderer.** The host sends start times; the renderer shows elapsed time and spinners and settles them itself. A host should never need a `setInterval`.
6. **Contract safety:** a version check at startup, a visible warning for unknown or invalid events, TypeScript types generated from the Rust types, and helpers that reject when the renderer exits. Nothing gets dropped silently.
7. **Migration path from Ink:** a thin adapter that maps autopilot's existing `ChatEvent` model onto Camouflage events, so the migration is one module and the two UIs can run side by side behind `--ui`.
8. **Autopilot's real traffic is the conformance suite.** Record real autopilot sessions as `.camo` files and replay them in CI (golden screen snapshots plus the idle soak test).
9. **Distribution that just works:** prebuilt binaries for macOS, Linux and eventually Windows; a stable dist-tag, no prerelease-range pinning traps; a clear error message when the binary is missing.

## 7. Plan

**A. autopilot hotfix** (branch `fix/idle-cpu`, worktree `~/autopilot-idle-cpu`, based on `origin/main` 1.2.1; not committed yet)
- [x] A1. Finished chat events go through `<Static>` (`src/ui/chat.tsx`): everything before the first unsettled event (a streaming reply, a running or queued tool, a queued user message) is printed once. `<Static>` remounts when the event list is replaced (`/clear`, `/resume`, compaction). `/clear` now clears the visible screen itself (`src/ui/slash-commands.ts`).
- [x] A2. `src/ui/spinner.tsx` replaces `ink-spinner` everywhere: one shared 100 ms clock that exists only while a spinner is mounted. Turn cleanup (`src/app.tsx`) now settles every streaming reply and every running or queued tool, not just the active reply.
- [x] A3. `src/util/atomic-file.ts`: temp-file-and-rename writes for `usage.json` and `history.jsonl`, a lock across processes (a lock directory, broken after 15 s if stale), and recovery of a corrupted `usage.json` (the broken file is kept as `usage.json.corrupt-<ts>`, the valid prefix is salvaged).
- [~] A4. Component-level soak test passes (`src/ui/chat-idle.test.tsx`), see the numbers in the work log. A soak of the real app is still pending: run the branch build as your daily driver for a day and compare CPU and memory.

Known limits of the Ink hotfix (the Camouflage rewrite removes them):
- Printed events are final. Toggling verbose or reasoning, or the repeated-call marker, only affects events printed afterwards.
- While a single streaming reply is taller than the screen, Ink still clears and reprints everything on each frame. That costs CPU while streaming, not while idle. The fix is to print finished paragraphs as they complete (§3, rule 1).
- Ink keeps its own copy of all printed output and reprints it on some redraws (for example a resize), so a `/clear`ed screen can come back after a resize.

**B. Camouflage rewrite**
- [~] B1. Inline renderer: printed output plus a small live area (§3). Done behind `--ui inline` (#31 writer, #32 blocks and markdown, #33 editor and chrome, #34 session, #35 binary). Left: make it the default; turn full-screen mode into the Ctrl+O transcript/replay view.
- [~] B2. Event-driven loop with no fixed ticker, settle on turn end, skip identical frames: done (#34, #35); waiting on a prompt counts as idle. Left: slow down when the terminal is unfocused.
- [x] B3. Input (#33, #35): crossterm with `use-dev-tty`, movement by grapheme, UTF-8, `\`+Enter / Shift+Enter / Alt+Enter, undo and yank, draft kept while browsing history, paste chips, no single-key shortcuts.
- [x] B4. Markdown with pulldown-cmark, syntax highlighting with syntect mapped onto the terminal palette (#32).
- [ ] B5. Protocol: row ids and updates, a generic block type, host-set theme, accent and keymap, a version check, a warning for unknown events.
- [ ] B6. SDK: TypeScript types generated from the Rust types; helpers reject when the renderer exits; `renderToTerminal` by default; capture the host's console output.
- [ ] B7. Fix the smaller bugs in §1.1 (forms, panic handler, crash dump location, search over SQLite).
- [~] B8. End-to-end pseudo-terminal tests and the idle soak (#35). Left: record real autopilot sessions as fixtures.

## Work log

- **2026-09-28** — Audit, commit-history review and visual review done. Prototype published. Idle-CPU investigation: confirmed the whole-transcript redraw and a spinner that keeps animating; found the `usage.json` corruption. Created the `fix/idle-cpu` worktree.
- **2026-09-29** — Added §6 (adoption; autopilot is the first customer). Implemented A1–A3 on `fix/idle-cpu`.
  - Measured with a 2,000-turn transcript and one visible spinner: the old `ChatView` wrote **727,785 bytes per frame** (the whole history, about 10 times a second); the new one writes **61 bytes**. With no spinner the idle UI writes nothing and runs no timer.
  - Full suite: 904/905 pass. The one failure (`theme-contrast.test.ts`) fails on a clean `main` too.
  - Found that `history.jsonl` only had today's entry, which fits the same cross-process race (a read during another process's rewrite comes back empty). Backed up `usage.json` to `usage.json.bak-2026-09-29`.
  - Committed as autopilot PR #663 (squash-merge; release-please will pick it up as 1.2.2 under Performance Improvements). Waiting for review: merging needs the owner.
- **2026-09-29 (later)** — Camouflage inline renderer built as a stack of PRs, each waiting for review and merge (merge commits, as usual in this repo):
  - #30 this doc · #31 terminal writer · #32 transcript blocks and markdown · #33 editor and chrome · #34 session model · #35 `--ui inline` binary mode and end-to-end tests.
  - Driven end to end on a pseudo-terminal: typing capital letters, `—`, `café` and emoji works; the transcript stays in scrollback after exit; tables, code and inline diffs render as in the prototype.
  - Idle soak: 2.2 s more idle writes 0 bytes and causes 0 extra wakeups (the test fails with an injected 80 ms tick: 8 vs 35 wakeups).
  - Release build latency: ~6 ms from a host token to the screen, ~25 ms for a permission prompt.
  - #36 SDK: `mount({ ui: "inline" })`, `permission()` helper, helpers settle when the renderer exits, typed `send()`, and a drift test between the SDK types and the protocol enum.
  - The autopilot hotfix (#663) was merged and shipped in autopilot 1.2.2.
- **Resume here:**
  1. Merge the Camouflage stack in order: #30, #31, #32, #33, #34, #35, #36. Each PR's base is the one before it, so retarget to `main` as each lands. Then merge the release-please PR.
  2. Build the autopilot integration on branch `feat/camouflage-inline` in `~/autopilot-idle-cpu` (branched from 1.3.0; nothing written yet). Plan: a new `src/camouflage-mode.ts` built on `emit-mode.ts`, which already maps `runAgentTurn` callbacks to events. Reuse from the Ink app: tool titles (`humanizeToolTitle`, `render.title`/`render.diff` in `src/ui/tool-view.tsx`), `recordUsage`, `saveSessionSafe`, and the mode and system-prompt rebuild. Use `permission()` with diffs, Shift+Tab mode cycling, `CancelRequested` → abort, and core slash commands (/help /clear /model /compact /cost /exit). Re-enable `--ui camouflage` in `src/index.tsx`. Keep the glue under ~800 lines.
  3. Version trap: autopilot pins `camouflage-tui` at `^2.1.0-beta.1`, and a caret range on a prerelease won't match `2.2.0-beta.x`. Bump the range explicitly after the release.
  4. Make `--ui inline` the default in Camouflage once autopilot runs on it daily.
- **2026-09-30** — Shipped.
  - The stacked PRs #32–36 had merged into `feat/inline-writer`, not `main`; #38 landed them on `main`.
  - Wiring autopilot surfaced three bugs, all fixed: the SDK's default mode never started (bare `--emit-responses`, #40); a dead renderer crashed the host with EPIPE (#40); the keyboard-protocol probe wrote into the host's event stream and ate the first command (#43, with a regression test).
  - Releases: 2.2.0-beta.1 and 2.2.1-beta.1 were tagged but never reached npm (publish token rejected; binaries skipped). #42 moved publishing after the binary build, onto npm trusted publishing, with a manual re-publish trigger. **2.2.2-beta.1 is on npm with binaries.**
  - The idle soak now asserts zero timer wakeups; loop stats record wakeups by source.
  - autopilot #672 brings back `--ui camouflage` on `^2.2.2-beta.1`, verified with the published package in a pseudo-terminal (real model turn, `/help`, modes, cost, 0 bytes while idle, clean exit).
  - Release gotcha: release-please only tracks `sdk/node`, and has no option to add `crates/` (checked 17.11.2). Crate-only fixes need `scripts/trigger-binary-release.sh` (a `Release-As:` commit) to ship.
- **Next:** /compact and the MCP/LSP/memory managers in autopilot's Camouflage mode; daily-drive it; then make `--ui inline` the Camouflage default and `camouflage` the autopilot default.