use crate::{draw, input, tty};
use anyhow::{Context, Result};
use camouflage_headless::emit::{spawn_writer, OutgoingEvent};
use camouflage_headless::{run_reader, NdjsonDecoder};
use camouflage_protocol::{Event, EventType, SCHEMA_VERSION};
use camouflage_renderer::{reconstruct_rows, RenderModel, Row, ViewportState};
use camouflage_store::{EventStore, SqliteStore};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::Stdout;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::io::BufReader;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};
use uuid::Uuid;

pub struct Config {
    pub store: SqliteStore,
    pub stdin_events: bool,
    pub replay: Option<Uuid>,
    /// Play back a shareable .camo file (NDJSON, optionally prefixed with
    /// a `{"camo_version":...}` header line). Mutually exclusive with
    /// `stdin_events` and `replay`. Events are persisted to the store as
    /// a fresh session so the user can re-replay or export them again.
    pub play_file: Option<std::path::PathBuf>,
    pub fps: u32,
    pub row_cap: Option<usize>,
    pub emit_responses: bool,
    /// Optional pre-opened file descriptor for outbound NDJSON. When set,
    /// takes precedence over `emit_responses`. Host wires this up with
    /// e.g. `child_process.spawn(cmd, { stdio: [...,"pipe"] })` and points
    /// the renderer at the resulting fd number.
    pub responses_fd: Option<i32>,
    /// Header brand resolved by the binary (host name / cwd folder). Seeds
    /// the model's initial title before the first frame; empty means the
    /// header shows the session id alone.
    pub app_title: String,
}

struct HistoryReq {
    session: Uuid,
    from_seq: i64,
    to_seq: i64,
}

struct HistoryResp {
    from_seq: i64,
    rows: Vec<Row>,
}

/// State for `--replay <session>`. The events vector is the full session
/// loaded once at startup; the position is how many we've applied so far.
/// On each frame tick we advance the position by `speed_eps * speed_mult * dt`,
/// applying any newly-elapsed events to the render model.
struct ReplayState {
    events: Vec<Event>,
    position: usize,
    playing: bool,
    /// Events per second at 1.0 multiplier.
    speed_eps: f32,
    /// User-adjustable multiplier (0.25, 0.5, 1, 2, 4, 8, 16, 64).
    speed_mult: f32,
    /// Fractional events carried over between ticks.
    accumulator: f32,
    last_tick: tokio::time::Instant,
}

impl ReplayState {
    fn is_complete(&self) -> bool {
        self.position >= self.events.len()
    }

    /// Apply `n` events forward, returning a count of how many were applied.
    fn step_forward(&mut self, n: usize, model: &mut RenderModel) -> usize {
        let mut applied = 0;
        for _ in 0..n {
            if self.position >= self.events.len() {
                break;
            }
            model.apply(&self.events[self.position]);
            self.position += 1;
            applied += 1;
        }
        applied
    }

    /// Step backward by `n` events by rebuilding the model from scratch up to
    /// position - n. O(N) — fine for debug/inspect use.
    fn step_backward(&mut self, n: usize, model: &mut RenderModel) {
        let target = self.position.saturating_sub(n);
        // Preserve the resolved header brand across the rebuild — it's seeded
        // from the binary (--app-title / cwd), not necessarily re-derivable
        // from the replayed events alone.
        let title = model.app_title().to_string();
        *model = RenderModel::new();
        model.set_app_title(&title);
        for ev in &self.events[..target] {
            model.apply(ev);
        }
        self.position = target;
    }

    fn bump_speed(&mut self, faster: bool) {
        let levels = [0.25_f32, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 64.0];
        let idx = levels
            .iter()
            .position(|&v| (v - self.speed_mult).abs() < 0.01)
            .unwrap_or(2);
        let new = if faster {
            (idx + 1).min(levels.len() - 1)
        } else {
            idx.saturating_sub(1)
        };
        self.speed_mult = levels[new];
    }
}

pub async fn run(cfg: Config) -> Result<()> {
    let store = Arc::new(cfg.store);
    let session_id = cfg.replay.unwrap_or_else(Uuid::new_v4);

    let mut model = match cfg.row_cap {
        Some(c) => RenderModel::with_cap(c),
        None => RenderModel::new(),
    };
    model.set_app_title(&cfg.app_title);
    // v0.2 inspector state. When open, a side panel shows the raw JSON of
    // the event under the inspector cursor. We look up the event from the
    // store by seq on demand and cache the pretty-printed JSON. Cursor
    // offset is rows-from-the-newest, so 0 = bottom-most row.
    let mut inspector_open: bool = false;
    // v0.4: help-overlay toggle (`?` when input buf is empty).
    let mut help_open: bool = false;
    let mut inspector_cursor: usize = 0;
    let mut inspector_cached_seq: Option<i64> = None;
    let mut inspector_cached_json: String = String::new();
    // v0.2 filter toolbar — cycles through row-kind subsets.
    let mut row_filter = RowFilter::All;
    // v0.2 search — set of matching seqs + index of the current focus.
    let mut search_open: bool = false;
    let mut search_query: String = String::new();
    let mut search_matches: Vec<i64> = Vec::new();
    let mut search_current: usize = 0;
    // v0.2 bookmarks — vim-style 'm' / `'` to add and cycle through.
    let mut bookmarks: Vec<i64> = Vec::new();
    let mut bookmark_cursor: usize = 0;
    let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
    let mut viewport = ViewportState::new(session_id, height.saturating_sub(4), width);
    let mut input_buf = String::new();
    // Character index of the insertion point in input_buf. Mutated by
    // handle_key (cursor-aware: Left/Right/Home/End/WordLeft/WordRight)
    // and by the renderer when filling input on slash/mention selection.
    let mut input_cursor: usize = 0;
    let mut status: String = "idle".into();

    // v0.4: live-metrics overlay state. Tracks total events seen, a 1-second
    // rolling rate, and the most-recent draw frame time. Cheap; updated
    // inline in the event-receive and render hot paths.
    let mut metrics_open: bool = false;
    let session_started_at = std::time::Instant::now();
    let mut total_events: u64 = 0;
    let mut events_since_window: u64 = 0;
    let mut events_per_sec: f64 = 0.0;
    let mut last_rate_window = std::time::Instant::now();
    let mut last_frame_time_us: u128 = 0;

    // v0.4: active theme name (cycled with `T`).
    let mut theme_name: String = "default-dark".to_string();
    // v0.4.5: tool-output overlay toggle (X).
    let mut tool_output_open: bool = false;
    // v0.4.5: free-text feedback typed while a permission widget is shown.
    // Cleared each time a permission is resolved.
    let mut permission_feedback: String = String::new();
    // v0.4.5: slash-picker selection cursor (within the *filtered* list).
    let mut slash_picker_index: usize = 0;
    // Sub-picker for slash-command discrete arguments. When Enter is
    // pressed on a top-level command whose args_hint parses as a
    // pipe-separated set of bare options (e.g. /mode → "edit|plan|auto"),
    // we drop into a second-level picker for the user to choose one
    // without typing. (parent_name, options, optional, selected_index).
    let mut slash_sub_state: Option<(String, Vec<String>, bool, usize)> = None;
    // v0.4.5: @-mention picker selection cursor.
    let mut mention_picker_index: usize = 0;
    // v0.5.1+: scroll offset for the todo checklist panel. Auto-updated by
    // the draw layer so the first in-progress task is always visible.
    let mut todo_scroll_offset: usize = 0;
    // v0.4.6: per-session input history. Each submitted user prompt is
    // pushed; Up/Down at the input prompt walks through them. `index` is
    // the cursor (None = "live" buffer, not browsing history).
    let mut input_history: Vec<String> = Vec::new();
    let mut input_history_index: Option<usize> = None;
    // v0.5+: multi-line paste preview. When a paste contains newlines or
    // exceeds 500 chars, we show a preview overlay and require confirmation
    // before inserting it into the input buffer.
    let mut paste_preview: Option<String> = None;
    // Timestamp of the most recent Ctrl+C. Used to implement the
    // "press Ctrl+C twice to exit" semantics: a lone Ctrl+C emits
    // CancelRequested (interrupt the agent) and arms this timestamp;
    // a second Ctrl+C within CTRL_C_DOUBLE_PRESS_WINDOW disarms and
    // shuts the app down.
    let mut last_ctrl_c: Option<std::time::Instant> = None;
    // Mouse-capture toggle (Shift+S). Default ON because the wheel-scroll
    // UX is more important than text selection — but users on terminals
    // without Option/Shift-click bypass need a way to turn it off so they
    // can copy snippets with their mouse. Initialised to true to match
    // setup_terminal()'s emitted sequence.
    let mut mouse_capture_on = true;
    const CTRL_C_DOUBLE_PRESS_WINDOW: std::time::Duration =
        std::time::Duration::from_millis(1500);

    // Replay state: loaded but not played-through. We start paused at
    // position 0 so the user can scrub controls before content shows.
    let mut replay_state: Option<ReplayState> = None;
    if let Some(sid) = cfg.replay {
        let events = store.load_session(sid).context("loading session")?;
        let n = events.len();
        replay_state = Some(ReplayState {
            events,
            position: 0,
            playing: true,        // start auto-playing; Space pauses
            speed_eps: 50.0,      // events per second at 1x; user can +/- adjust
            speed_mult: 1.0,
            accumulator: 0.0,
            last_tick: tokio::time::Instant::now(),
        });
        status = format!("replay 0/{n}");
    }

    let start_seq = store.latest_seq(session_id).unwrap_or(-1).max(-1) + 1;
    let seq_counter = Arc::new(AtomicI64::new(start_seq));

    // Outbound NDJSON emitter (renderer → host). Three sinks possible:
    //   --responses-fd N → writes to fd N (host owns the fd; typical when
    //                      stdout is reserved for rendering to the user's
    //                      terminal and we're spawned as a child process)
    //   --emit-responses=true (or default with --stdin-events) → stdout
    //   neither           → no outbound emission
    let outbound_tx: Option<mpsc::Sender<OutgoingEvent>> = if let Some(fd) = cfg.responses_fd {
        let (tx, rx) = mpsc::channel::<OutgoingEvent>(64);
        // SAFETY: the host opened fd N before spawning us via stdio
        // configuration. The File closes the fd on drop — that's the
        // desired behaviour at process exit (host sees EOF on its
        // pipe-read side). spawn_writer owns the File through the
        // BufWriter wrapper for the task's lifetime.
        use std::os::unix::io::FromRawFd;
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        spawn_writer(file, rx);
        Some(tx)
    } else if cfg.emit_responses {
        let (tx, rx) = mpsc::channel::<OutgoingEvent>(64);
        spawn_writer(std::io::stdout(), rx);
        Some(tx)
    } else {
        None
    };

    // NDJSON ingestion from stdin (pipe).
    let (ev_tx, mut ev_rx) = mpsc::channel::<Event>(4096);
    if cfg.stdin_events {
        let decoder = NdjsonDecoder::new(session_id);
        let stdin = tokio::io::stdin();
        let reader = BufReader::new(stdin);
        let tx = ev_tx.clone();
        tokio::spawn(async move {
            let _ = run_reader(reader, decoder, tx).await;
        });
    }
    if let Some(path) = cfg.play_file.clone() {
        // .camo container = optional header line `{"camo_version":...}`
        // followed by NDJSON events. We detect and strip the header here,
        // then hand the remaining stream to the same NDJSON decoder as
        // --stdin-events so the rest of the pipeline is unchanged.
        let decoder = NdjsonDecoder::new(session_id);
        let tx = ev_tx.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let file = match tokio::fs::File::open(&path).await {
                Ok(f) => f,
                Err(e) => {
                    tracing::error!(?e, "open .camo file");
                    return;
                }
            };
            let mut reader = BufReader::new(file);
            // Peek the first line. If it's a camo header object, skip
            // it; otherwise feed it back into the decoder as the first
            // event line.
            let mut first = String::new();
            let n = reader.read_line(&mut first).await.unwrap_or(0);
            let header_consumed = n > 0
                && serde_json::from_str::<serde_json::Value>(first.trim())
                    .ok()
                    .and_then(|v| v.get("camo_version").cloned())
                    .is_some();
            if !header_consumed && !first.trim().is_empty() {
                if let Ok(ev) = decoder.parse_line(first.trim()) {
                    let _ = tx.send(ev).await;
                }
            }
            let _ = run_reader(reader, decoder, tx).await;
        });
    }
    drop(ev_tx); // we won't send into ev_tx ourselves.

    // Persist-before-render writer task.
    let (persist_tx, mut persist_rx) = mpsc::channel::<Event>(4096);
    let (rendered_tx, mut rendered_rx) = mpsc::channel::<Event>(4096);
    {
        let store = store.clone();
        tokio::spawn(async move {
            let mut batch: Vec<Event> = Vec::with_capacity(256);
            let mut flush = interval(Duration::from_millis(16));
            flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    maybe = persist_rx.recv() => {
                        match maybe {
                            Some(ev) => batch.push(ev),
                            None => break,
                        }
                        while let Ok(ev) = persist_rx.try_recv() {
                            batch.push(ev);
                            if batch.len() >= 512 { break; }
                        }
                        if batch.len() >= 256 {
                            if let Err(e) = store.append_batch(&batch) {
                                tracing::warn!(?e, "store append failed");
                            }
                            for ev in batch.drain(..) {
                                if rendered_tx.send(ev).await.is_err() { return; }
                            }
                        }
                    }
                    _ = flush.tick() => {
                        if !batch.is_empty() {
                            if let Err(e) = store.append_batch(&batch) {
                                tracing::warn!(?e, "store append failed");
                            }
                            for ev in batch.drain(..) {
                                if rendered_tx.send(ev).await.is_err() { return; }
                            }
                        }
                    }
                }
            }
            if !batch.is_empty() {
                let _ = store.append_batch(&batch);
                for ev in batch.drain(..) {
                    let _ = rendered_tx.send(ev).await;
                }
            }
        });
    }

    // Bridge: ev_rx (stdin) → persist queue with renormalised seq.
    {
        let seq_counter = seq_counter.clone();
        let persist_tx = persist_tx.clone();
        tokio::spawn(async move {
            while let Some(mut ev) = ev_rx.recv().await {
                ev.session_id = session_id;
                ev.seq = seq_counter.fetch_add(1, Ordering::Relaxed);
                if ev.schema_version == 0 {
                    ev.schema_version = SCHEMA_VERSION;
                }
                if persist_tx.send(ev).await.is_err() {
                    break;
                }
            }
        });
    }

    // Terminal setup. crossterm's raw mode uses tcsetattr — works regardless
    // of what stdin is.
    let mut terminal = setup_terminal().context("setup terminal")?;
    // Wire the crash-replay ring buffer BEFORE installing the panic hook
    // so any in-startup panic still gets a dump.
    let _ = CRASH_RING.set(std::sync::Arc::new(std::sync::Mutex::new(
        std::collections::VecDeque::with_capacity(CRASH_RING_CAP),
    )));
    install_panic_hook();

    // Open /dev/tty for reading and start our custom key reader. We use this
    // instead of crossterm::event::poll because the latter requires
    // registering /dev/tty with kqueue on macOS, which returns EINVAL.
    let tty_fd = tty::open_tty_for_read().context("opening /dev/tty for keys")?;
    let key_rx = tty::spawn_key_reader(tty_fd);

    let frame_period = Duration::from_secs_f64(1.0 / cfg.fps as f64);
    let mut ticker = interval(frame_period);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Spinner-driving frame counter. We need redraws even when the model isn't
    // dirty while a spinner is on screen, so the ticker also bumps this and
    // forces a draw if anything spinnable is active.
    let mut frame_counter: u64 = 0;

    // History backfill worker. When the user scrolls past the in-memory
    // window we ask this worker for a chunk of older rows. Only one request
    // is in flight at a time (gated by `history_inflight`).
    const HISTORY_CHUNK: i64 = 500;
    let (history_req_tx, mut history_req_rx) = mpsc::channel::<HistoryReq>(4);
    let (history_done_tx, mut history_done_rx) = mpsc::channel::<HistoryResp>(4);
    {
        let store = store.clone();
        tokio::spawn(async move {
            while let Some(req) = history_req_rx.recv().await {
                let store = store.clone();
                let events = tokio::task::spawn_blocking(move || {
                    store.load_range(req.session, req.from_seq, req.to_seq)
                })
                .await;
                let rows = match events {
                    Ok(Ok(evs)) => reconstruct_rows(&evs),
                    Ok(Err(e)) => {
                        tracing::warn!(?e, "history load_range failed");
                        Vec::new()
                    }
                    Err(e) => {
                        tracing::warn!(?e, "history worker join failed");
                        Vec::new()
                    }
                };
                if history_done_tx
                    .send(HistoryResp { from_seq: req.from_seq, rows })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
    }
    let mut history_inflight = false;

    // SessionStarted on fresh sessions.
    if cfg.replay.is_none() {
        let ev = Event {
            id: Uuid::new_v4(),
            session_id,
            seq: seq_counter.fetch_add(1, Ordering::Relaxed),
            timestamp_ms: now_ms(),
            schema_version: SCHEMA_VERSION,
            event_type: EventType::SessionStarted,
            payload: serde_json::json!({}),
        };
        let _ = persist_tx.send(ev).await;
    }

    loop {
        // Non-blocking poll for keys before each frame.
        while let Ok(key) = key_rx.try_recv() {
            if matches!(key, crate::tty::Key::Esc) {
                esc_log("key Esc received");
            }
            // Esc parity with Ctrl+C: when there's a live turn in flight,
            // Esc must abort it from ANY context. Ctrl+C reaches its abort
            // (Action::Quit) from everywhere because nothing below
            // short-circuits it; Esc, by contrast, gets swallowed by the
            // overlay-dismiss / component-modal / inspector handlers that
            // follow (they consume Esc to close a popup or cancel a modal
            // and `continue` before it ever reaches the CancelStream abort
            // path). That made Esc feel like it "doesn't abort" while
            // Ctrl+C does. We only pre-empt when there is actually
            // something to abort, so an Esc meant to dismiss a popup or
            // cancel a modal at idle still does exactly that.
            if matches!(key, crate::tty::Key::Esc) && model.has_inflight_work() {
                esc_log("Esc with in-flight work → abort (pre-empting overlay/modal handlers)");
                emit_cancel_requested(
                    &outbound_tx,
                    &mut model,
                    session_id,
                    &seq_counter,
                    &mut permission_feedback,
                )
                .await;
                status = "canceled".into();
                model.mark_dirty();
                continue;
            }
            // Sub-picker short-circuit: when the user picked a top-level
            // slash command whose args parse as a discrete option list,
            // we display a second picker and consume Up/Down/Enter/Esc
            // here so they can choose without typing.
            if let Some(state) = slash_sub_state.clone() {
                use crate::tty::Key;
                let (parent, options, optional, idx) = state;
                let total = options.len() + if optional { 1 } else { 0 };
                match key {
                    Key::Up => {
                        let new = if idx == 0 { total.saturating_sub(1) } else { idx - 1 };
                        slash_sub_state = Some((parent, options, optional, new));
                        model.mark_dirty();
                        continue;
                    }
                    Key::Down => {
                        let new = if total == 0 { 0 } else { (idx + 1) % total };
                        slash_sub_state = Some((parent, options, optional, new));
                        model.mark_dirty();
                        continue;
                    }
                    Key::Esc | Key::Backspace | Key::Left => {
                        // Back to the top-level picker. input_buf still
                        // holds "/parent" so the parent picker filters
                        // sensibly when the overlay re-renders. Left is
                        // included so the user's natural "go back" key on
                        // a nested list maps to the obvious thing.
                        slash_sub_state = None;
                        model.mark_dirty();
                        continue;
                    }
                    Key::Enter => {
                        let text = if optional && idx == options.len() {
                            format!("/{}", parent)
                        } else {
                            format!("/{} {}", parent, options[idx])
                        };
                        slash_sub_state = None;
                        input_buf.clear();
                        input_cursor = 0;
                        if input_history.last().map(|s| s.as_str()) != Some(text.as_str()) {
                            input_history.push(text.clone());
                            if input_history.len() > 200 {
                                input_history.remove(0);
                            }
                        }
                        input_history_index = None;
                        if let Some(tx) = outbound_tx.as_ref() {
                            let ev = Event {
                                id: Uuid::new_v4(),
                                session_id,
                                seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                                timestamp_ms: now_ms(),
                                schema_version: SCHEMA_VERSION,
                                event_type: EventType::UserInputSubmitted,
                                payload: serde_json::json!({"text": text}),
                            };
                            let _ = tx.send(OutgoingEvent(ev)).await;
                        } else {
                            let ev = Event {
                                id: Uuid::new_v4(),
                                session_id,
                                seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                                timestamp_ms: now_ms(),
                                schema_version: SCHEMA_VERSION,
                                event_type: EventType::UserMessageCreated,
                                payload: serde_json::json!({"text": text}),
                            };
                            let _ = persist_tx.send(ev).await;
                        }
                        model.mark_dirty();
                        continue;
                    }
                    _ => {
                        // Any other keypress: drop sub-picker and let the
                        // normal handler take over (so manual typing of an
                        // argument still works as an escape hatch).
                        slash_sub_state = None;
                    }
                }
            }
            // Slash-picker short-circuit: when the input starts with `/` and
            // the host has registered slash commands, ↑/↓/Enter drive the
            // picker instead of falling through to the normal handler.
            let slash_active = input_buf.starts_with('/')
                && !model.slash_commands().is_empty()
                && !search_open
                && model.pending_permission().is_none();
            if slash_active {
                let needle: String = input_buf.chars().skip(1).collect();
                let matches: Vec<&camouflage_renderer::model::SlashCmdEntry> = model
                    .slash_commands()
                    .iter()
                    .filter(|c| c.name.starts_with(&needle))
                    .collect();
                if !matches.is_empty() {
                    match key {
                        // Wrap on both ends so power-users can fly to the
                        // bottom of a long list by pressing Up once.
                        crate::tty::Key::Up => {
                            slash_picker_index = if slash_picker_index == 0 {
                                matches.len() - 1
                            } else {
                                slash_picker_index - 1
                            };
                            model.mark_dirty();
                            continue;
                        }
                        crate::tty::Key::Down => {
                            slash_picker_index = (slash_picker_index + 1) % matches.len();
                            model.mark_dirty();
                            continue;
                        }
                        crate::tty::Key::Enter => {
                            let idx = slash_picker_index.min(matches.len() - 1);
                            let chosen = matches[idx];
                            let name = chosen.name.clone();
                            slash_picker_index = 0;
                            // Decide what to do based on the args_hint:
                            //  - no hint           → submit "/name" directly.
                            //  - discrete options  → drop into the sub-picker.
                            //  - free-text args    → fall back to inserting
                            //    "/name " into the buffer so the user can
                            //    type the argument (preserves the manual
                            //    escape hatch the user explicitly asked for).
                            let parsed = chosen
                                .args_hint
                                .as_deref()
                                .and_then(parse_discrete_args);
                            match (chosen.args_hint.as_deref(), parsed) {
                                (None, _) => {
                                    let text = format!("/{}", name);
                                    input_buf.clear();
                                    input_cursor = 0;
                                    if input_history.last().map(|s| s.as_str())
                                        != Some(text.as_str())
                                    {
                                        input_history.push(text.clone());
                                        if input_history.len() > 200 {
                                            input_history.remove(0);
                                        }
                                    }
                                    input_history_index = None;
                                    if let Some(tx) = outbound_tx.as_ref() {
                                        let ev = Event {
                                            id: Uuid::new_v4(),
                                            session_id,
                                            seq: seq_counter
                                                .fetch_add(1, Ordering::Relaxed),
                                            timestamp_ms: now_ms(),
                                            schema_version: SCHEMA_VERSION,
                                            event_type: EventType::UserInputSubmitted,
                                            payload: serde_json::json!({"text": text}),
                                        };
                                        let _ = tx.send(OutgoingEvent(ev)).await;
                                    } else {
                                        let ev = Event {
                                            id: Uuid::new_v4(),
                                            session_id,
                                            seq: seq_counter
                                                .fetch_add(1, Ordering::Relaxed),
                                            timestamp_ms: now_ms(),
                                            schema_version: SCHEMA_VERSION,
                                            event_type: EventType::UserMessageCreated,
                                            payload: serde_json::json!({"text": text}),
                                        };
                                        let _ = persist_tx.send(ev).await;
                                    }
                                }
                                (Some(_), Some((options, optional))) => {
                                    input_buf = format!("/{}", name);
                                    input_cursor = input_buf.chars().count();
                                    slash_sub_state = Some((name, options, optional, 0));
                                }
                                (Some(_), None) => {
                                    input_buf = format!("/{} ", name);
                                    input_cursor = input_buf.chars().count();
                                }
                            }
                            model.mark_dirty();
                            continue;
                        }
                        _ => { /* fall through to normal handler */ }
                    }
                }
                // Reset selection any time the matches set shrinks below the
                // current index (e.g. user typed more chars).
                if slash_picker_index >= matches.len().max(1) {
                    slash_picker_index = 0;
                }
            } else {
                slash_picker_index = 0;
            }
            // Input-history walk: Up/Down when the input prompt has focus,
            // no overlays/pickers are open, no permission is pending, no
            // search is active, no replay scrubber is taking the key.
            // Only walks history when the input buffer is empty OR the
            // user is already in the middle of a history walk — otherwise
            // Up/Down with typed text would silently clobber the user's
            // unsubmitted input. Matches Ink's readline-style convention.
            // Compute mention_active locally so input_focused can include
            // it (the main mention_active computation runs below for its
            // own ↑/↓ short-circuit; cheap to evaluate twice).
            let mention_active_for_focus = !search_open
                && model.pending_permission().is_none()
                && !model.mention_candidates().is_empty()
                && last_mention_partial(&input_buf).is_some();
            let input_focused = !search_open
                && model.pending_permission().is_none()
                && replay_state.is_none()
                && !inspector_open
                && !slash_active
                && !mention_active_for_focus
                && !help_open
                && !metrics_open
                && !tool_output_open
                && model.active_select_list().is_none()
                && model.active_confirm().is_none()
                && model.active_form().is_none();
            // Shift+Tab fires the mode-cycle regardless of input contents —
            // it's not a typing key and the user expects it to work mid-
            // typing. Tab when input is empty also fires mode-cycle; Tab
            // inside text falls through to form-step navigation (the only
            // place Tab is plausibly a "next field" rather than "cycle
            // mode" is inside a Form modal, which has its own handler
            // upstream of here).
            if input_focused {
                match key {
                    crate::tty::Key::BackTab => {
                        if let Some(tx) = outbound_tx.as_ref() {
                            let ev = Event {
                                id: Uuid::new_v4(),
                                session_id,
                                seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                                timestamp_ms: now_ms(),
                                schema_version: SCHEMA_VERSION,
                                event_type: EventType::ModeChangeRequested,
                                payload: serde_json::json!({ "direction": "prev" }),
                            };
                            let _ = tx.send(OutgoingEvent(ev)).await;
                        }
                        continue;
                    }
                    crate::tty::Key::Tab if input_buf.is_empty() => {
                        if let Some(tx) = outbound_tx.as_ref() {
                            let ev = Event {
                                id: Uuid::new_v4(),
                                session_id,
                                seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                                timestamp_ms: now_ms(),
                                schema_version: SCHEMA_VERSION,
                                event_type: EventType::ModeChangeRequested,
                                payload: serde_json::json!({ "direction": "next" }),
                            };
                            let _ = tx.send(OutgoingEvent(ev)).await;
                        }
                        continue;
                    }
                    _ => {}
                }
            }
            if input_focused && !input_history.is_empty() {
                match key {
                    crate::tty::Key::Up => {
                        let next = match input_history_index {
                            None => input_history.len() - 1,
                            Some(i) => i.saturating_sub(1),
                        };
                        input_history_index = Some(next);
                        input_buf = input_history[next].clone();
                        input_cursor = input_buf.chars().count();
                        model.mark_dirty();
                        continue;
                    }
                    crate::tty::Key::Down => {
                        match input_history_index {
                            Some(i) if i + 1 < input_history.len() => {
                                input_history_index = Some(i + 1);
                                input_buf = input_history[i + 1].clone();
                                input_cursor = input_buf.chars().count();
                            }
                            Some(_) => {
                                // Past the last entry → back to a fresh buffer.
                                input_history_index = None;
                                input_buf.clear();
                                input_cursor = 0;
                            }
                            None => { /* nothing to do */ }
                        }
                        model.mark_dirty();
                        continue;
                    }
                    _ => {}
                }
            }
            // Esc closes any open overlay before anything else gets it. This
            // makes the dismiss UX consistent: ?, M, X, T → Esc closes.
            // Also covers CC-6 Table (display-only modal — Esc is the only
            // exit).
            if matches!(key, crate::tty::Key::Esc) {
                if help_open || metrics_open || tool_output_open
                    || model.active_table().is_some()
                    || model.active_kv().is_some()
                {
                    help_open = false;
                    metrics_open = false;
                    tool_output_open = false;
                    if model.active_table().is_some() {
                        model.clear_table();
                    }
                    if model.active_kv().is_some() {
                        model.clear_kv();
                    }
                    model.mark_dirty();
                    continue;
                }
            }
            // CC-5 — Form short-circuit. Tab cycles fields; Shift+Tab
            // (Backtab) goes backward; Enter on last field submits;
            // Backspace edits; printable chars append; Esc cancels (when
            // allow_cancel).
            if model.active_form().is_some() {
                use crate::tty::Key;
                let (id, submit_values, cancel): (Option<String>, Option<std::collections::BTreeMap<String, String>>, bool) = {
                    let f = model.active_form().unwrap();
                    match key {
                        // Down or Tab-char → next field; Up → previous field.
                        Key::Down | Key::Char('\t') => {
                            if let Some(fm) = model.active_form_mut() {
                                fm.focused = (fm.focused + 1) % fm.fields.len();
                            }
                            model.mark_dirty();
                            continue;
                        }
                        Key::Up => {
                            if let Some(fm) = model.active_form_mut() {
                                fm.focused = if fm.focused == 0 {
                                    fm.fields.len() - 1
                                } else {
                                    fm.focused - 1
                                };
                            }
                            model.mark_dirty();
                            continue;
                        }
                        Key::Enter => {
                            // Submit if on last field, else advance.
                            if f.focused + 1 < f.fields.len() {
                                if let Some(fm) = model.active_form_mut() {
                                    fm.focused += 1;
                                }
                                model.mark_dirty();
                                continue;
                            }
                            let values: std::collections::BTreeMap<String, String> = f
                                .fields
                                .iter()
                                .map(|fld| (fld.name.clone(), fld.value.clone()))
                                .collect();
                            (Some(f.id.clone()), Some(values), false)
                        }
                        Key::Esc if f.allow_cancel => (Some(f.id.clone()), None, true),
                        Key::Backspace => {
                            if let Some(fm) = model.active_form_mut() {
                                if let Some(field) = fm.fields.get_mut(fm.focused) {
                                    field.value.pop();
                                }
                            }
                            model.mark_dirty();
                            continue;
                        }
                        Key::Char(c) => {
                            if let Some(fm) = model.active_form_mut() {
                                if let Some(field) = fm.fields.get_mut(fm.focused) {
                                    field.value.push(c);
                                }
                            }
                            model.mark_dirty();
                            continue;
                        }
                        _ => continue,
                    }
                };
                if let Some(id) = id {
                    let wizard_step = model.wizard_current_step_id().map(str::to_string);
                    let owned_by_wizard = wizard_step.as_deref() == Some(id.as_str());
                    if owned_by_wizard {
                        model.clear_form();
                        if cancel {
                            emit_wizard_cancelled(&outbound_tx, &mut model, session_id, &seq_counter).await;
                        } else if let Some(v) = submit_values {
                            advance_wizard_after(
                                &outbound_tx, &mut model, session_id, &seq_counter,
                                camouflage_renderer::model::WizardStepResult::Form(v),
                            ).await;
                        }
                        continue;
                    }
                    if let Some(tx) = outbound_tx.as_ref() {
                        let payload = if cancel {
                            serde_json::json!({ "id": id, "cancelled": true })
                        } else {
                            serde_json::json!({ "id": id, "values": submit_values })
                        };
                        let ev = Event {
                            id: Uuid::new_v4(),
                            session_id,
                            seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                            timestamp_ms: now_ms(),
                            schema_version: SCHEMA_VERSION,
                            event_type: EventType::FormResponse,
                            payload,
                        };
                        let _ = tx.send(OutgoingEvent(ev)).await;
                    }
                    model.clear_form();
                    continue;
                }
                continue;
            }
            // CC-2 — Confirm short-circuit. Y/N (case-insensitive) decides
            // immediately; ← / → toggles selection; Enter submits the
            // currently-selected button; Esc cancels (when allow_cancel).
            if model.active_confirm().is_some() {
                use crate::tty::Key;
                let (id, value, cancel): (Option<String>, Option<bool>, bool) = {
                    let c = model.active_confirm().unwrap();
                    match key {
                        Key::Char('y') | Key::Char('Y') => (Some(c.id.clone()), Some(true), false),
                        Key::Char('n') | Key::Char('N') => (Some(c.id.clone()), Some(false), false),
                        Key::Left | Key::Right => {
                            if let Some(cm) = model.active_confirm_mut() {
                                cm.selected_yes = !cm.selected_yes;
                            }
                            model.mark_dirty();
                            continue;
                        }
                        Key::Enter => (Some(c.id.clone()), Some(c.selected_yes), false),
                        Key::Esc if c.allow_cancel => (Some(c.id.clone()), None, true),
                        _ => continue,
                    }
                };
                if let Some(id) = id {
                    // CC-4 — if a wizard owns this step, intercept the
                    // response into the wizard's advance path instead of
                    // emitting ConfirmResponse externally.
                    let wizard_step = model.wizard_current_step_id().map(str::to_string);
                    let owned_by_wizard = wizard_step.as_deref() == Some(id.as_str());
                    if owned_by_wizard {
                        model.clear_confirm();
                        if cancel {
                            emit_wizard_cancelled(&outbound_tx, &mut model, session_id, &seq_counter).await;
                        } else if let Some(v) = value {
                            advance_wizard_after(
                                &outbound_tx, &mut model, session_id, &seq_counter,
                                camouflage_renderer::model::WizardStepResult::Confirm(v),
                            ).await;
                        }
                        continue;
                    }
                    if let Some(tx) = outbound_tx.as_ref() {
                        let payload = if cancel {
                            serde_json::json!({ "id": id, "cancelled": true })
                        } else {
                            serde_json::json!({ "id": id, "value": value })
                        };
                        let ev = Event {
                            id: Uuid::new_v4(),
                            session_id,
                            seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                            timestamp_ms: now_ms(),
                            schema_version: SCHEMA_VERSION,
                            event_type: EventType::ConfirmResponse,
                            payload,
                        };
                        let _ = tx.send(OutgoingEvent(ev)).await;
                    }
                    model.clear_confirm();
                    continue;
                }
                continue;
            }
            // CC-1 — SelectList short-circuit. When a SelectList is active,
            // all input flows into it: ↑/↓ navigate filtered options,
            // Enter submits the highlighted option, Esc cancels (when
            // allow_cancel), Backspace edits the filter, printable chars
            // append to the filter (when allow_filter).
            if model.active_select_list().is_some() {
                use crate::tty::Key;
                let (id, value_to_emit, cancel, filter_change): (Option<String>, Option<String>, bool, Option<(char, bool)>) = {
                    let sl = model.active_select_list().unwrap();
                    let visible = sl.filtered_indices();
                    match key {
                        Key::Up => {
                            // Wraps at the top edge so a single Up jumps
                            // to the bottom of the filtered list.
                            let pos = visible.iter().position(|&i| i == sl.selected).unwrap_or(0);
                            let next = if pos == 0 { visible.len() - 1 } else { pos - 1 };
                            if let Some(&new_sel) = visible.get(next) {
                                if let Some(s) = model.active_select_list_mut() {
                                    s.selected = new_sel;
                                }
                            }
                            model.mark_dirty();
                            continue;
                        }
                        Key::Down => {
                            let pos = visible.iter().position(|&i| i == sl.selected).unwrap_or(0);
                            let next = (pos + 1) % visible.len().max(1);
                            if let Some(&new_sel) = visible.get(next) {
                                if let Some(s) = model.active_select_list_mut() {
                                    s.selected = new_sel;
                                }
                            }
                            model.mark_dirty();
                            continue;
                        }
                        Key::Enter => {
                            // Submit: use the selected option if it's in the
                            // filtered set, otherwise first visible.
                            let chosen = if visible.contains(&sl.selected) {
                                sl.selected
                            } else if let Some(&first) = visible.first() {
                                first
                            } else {
                                // Empty filter — no-op rather than crash.
                                continue;
                            };
                            (Some(sl.id.clone()), Some(sl.options[chosen].value.clone()), false, None)
                        }
                        Key::Esc if sl.allow_cancel => {
                            (Some(sl.id.clone()), None, true, None)
                        }
                        Key::Backspace if sl.allow_filter => (None, None, false, Some(('\0', true))),
                        Key::Char(c) if sl.allow_filter => (None, None, false, Some((c, false))),
                        _ => continue,
                    }
                };
                if let Some(change) = filter_change {
                    if let Some(s) = model.active_select_list_mut() {
                        let (ch, backspace) = change;
                        if backspace { s.filter.pop(); } else { s.filter.push(ch); }
                        // After filtering, ensure selected is in the visible set;
                        // if not, snap to the first visible.
                        let visible = s.filtered_indices();
                        if !visible.contains(&s.selected) {
                            s.selected = *visible.first().unwrap_or(&0);
                        }
                    }
                    model.mark_dirty();
                    continue;
                }
                if let Some(id) = id {
                    let wizard_step = model.wizard_current_step_id().map(str::to_string);
                    let owned_by_wizard = wizard_step.as_deref() == Some(id.as_str());
                    if owned_by_wizard {
                        model.clear_select_list();
                        if cancel {
                            emit_wizard_cancelled(&outbound_tx, &mut model, session_id, &seq_counter).await;
                        } else if let Some(v) = value_to_emit {
                            advance_wizard_after(
                                &outbound_tx, &mut model, session_id, &seq_counter,
                                camouflage_renderer::model::WizardStepResult::Select(v),
                            ).await;
                        }
                        continue;
                    }
                    // Build + send SelectListResponse, then clear the modal.
                    if let Some(tx) = outbound_tx.as_ref() {
                        let payload = if cancel {
                            serde_json::json!({ "id": id, "cancelled": true })
                        } else {
                            serde_json::json!({ "id": id, "value": value_to_emit })
                        };
                        let ev = Event {
                            id: Uuid::new_v4(),
                            session_id,
                            seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                            timestamp_ms: now_ms(),
                            schema_version: SCHEMA_VERSION,
                            event_type: EventType::SelectListResponse,
                            payload,
                        };
                        let _ = tx.send(OutgoingEvent(ev)).await;
                    }
                    model.clear_select_list();
                    continue;
                }
                continue;
            }
            // @-mention picker short-circuit. Active when the last
            // whitespace-delimited token of input starts with '@' and the
            // host has registered candidates.
            let mention_active = !search_open
                && model.pending_permission().is_none()
                && !model.mention_candidates().is_empty()
                && last_mention_partial(&input_buf).is_some();
            if mention_active {
                let partial = last_mention_partial(&input_buf).unwrap_or_default();
                let matches: Vec<&camouflage_renderer::model::MentionEntry> = model
                    .mention_candidates()
                    .iter()
                    .filter(|m| m.token.contains(&partial))
                    .collect();
                if !matches.is_empty() {
                    match key {
                        crate::tty::Key::Up => {
                            mention_picker_index = if mention_picker_index == 0 {
                                matches.len() - 1
                            } else {
                                mention_picker_index - 1
                            };
                            model.mark_dirty();
                            continue;
                        }
                        crate::tty::Key::Down => {
                            mention_picker_index = (mention_picker_index + 1) % matches.len();
                            model.mark_dirty();
                            continue;
                        }
                        crate::tty::Key::Enter => {
                            let idx = mention_picker_index.min(matches.len() - 1);
                            // Replace the partial `@xxx` with the selected
                            // token (still prefixed with @) + a trailing
                            // space for readability.
                            if let Some(at_pos) = input_buf.rfind('@') {
                                input_buf.truncate(at_pos);
                                input_buf.push('@');
                                input_buf.push_str(&matches[idx].token);
                                input_buf.push(' ');
                                input_cursor = input_buf.chars().count();
                            }
                            mention_picker_index = 0;
                            model.mark_dirty();
                            continue;
                        }
                        _ => { /* fall through */ }
                    }
                }
                if mention_picker_index >= matches.len().max(1) {
                    mention_picker_index = 0;
                }
            } else {
                mention_picker_index = 0;
            }
            // v0.5+: paste preview short-circuit. When a large or multi-line
            // paste is pending confirmation, consume keys to accept/reject.
            if let Some(ref text) = paste_preview {
                match key {
                    crate::tty::Key::Enter | crate::tty::Key::Char('y') => {
                        for c in text.chars() {
                            input::insert_at_cursor(&mut input_buf, &mut input_cursor, c);
                        }
                        paste_preview = None;
                        model.mark_dirty();
                        continue;
                    }
                    crate::tty::Key::Esc | crate::tty::Key::Char('n') => {
                        paste_preview = None;
                        model.mark_dirty();
                        continue;
                    }
                    _ => {
                        // Ignore all other keys while the preview is open.
                        model.mark_dirty();
                        continue;
                    }
                }
            }
            // Priority order: search prompt → pending permission widget →
            // inspector cursor (if open) → replay controls (if --replay) →
            // normal handler.
            let mut action = if search_open {
                input::handle_key_search(key)
            } else if model.pending_permission().is_some() {
                input::handle_key_permission(key, &mut permission_feedback)
            } else if inspector_open
                && matches!(
                    key,
                    crate::tty::Key::Up | crate::tty::Key::Down | crate::tty::Key::Esc
                )
            {
                match key {
                    crate::tty::Key::Up => input::Action::InspectorCursorUp,
                    crate::tty::Key::Down => input::Action::InspectorCursorDown,
                    crate::tty::Key::Esc => input::Action::ToggleInspector,
                    _ => input::Action::None,
                }
            } else if let (Some(_), Some(act)) =
                (replay_state.as_ref(), input::handle_key_replay(key.clone(), &input_buf))
            {
                act
            } else if matches!(key, crate::tty::Key::Paste(ref t) if t.contains('\n') || t.len() > 500)
            {
                // Multi-line or large paste: show preview instead of inserting.
                if let crate::tty::Key::Paste(text) = key {
                    paste_preview = Some(text);
                    model.mark_dirty();
                }
                input::Action::None
            } else {
                input::handle_key(key, &mut input_buf, &mut input_cursor)
            };
            // Any input change (typed char, cursor move, backspace, Ctrl+W,
            // etc.) should trigger a redraw so the picker overlay /
            // cursor position update is visible immediately rather than
            // waiting for the next ticker frame.
            if matches!(action, input::Action::None) {
                model.mark_dirty();
            }
            match action {
                input::Action::Quit => {
                    // Ctrl+C is "interrupt the agent" on first press and
                    // "quit the app" on a second press within
                    // CTRL_C_DOUBLE_PRESS_WINDOW. Mirrors Ink / readline:
                    // users expect Ctrl+C to abort the running operation
                    // first and only exit when they really mean it. The
                    // timestamp expires naturally after the window, so
                    // pressing Ctrl+C once and then continuing to work is
                    // not a latent exit hazard.
                    let now = std::time::Instant::now();
                    let is_second_press = last_ctrl_c
                        .map(|t| now.duration_since(t) <= CTRL_C_DOUBLE_PRESS_WINDOW)
                        .unwrap_or(false);
                    if is_second_press {
                        return shutdown(&mut terminal, Ok(()));
                    }
                    last_ctrl_c = Some(now);
                    // Visible feedback: without this, a first Ctrl+C while
                    // idle looks like a no-op and the user assumes the TUI
                    // is broken.
                    status = "press Ctrl+C again to quit".into();
                    model.mark_dirty();
                    // First press interrupts the running turn — same abort
                    // path as Esc (deny open permission modal + emit
                    // CancelRequested + optimistic local cancel).
                    emit_cancel_requested(
                        &outbound_tx,
                        &mut model,
                        session_id,
                        &seq_counter,
                        &mut permission_feedback,
                    )
                    .await;
                    // Standalone mode (no host): the first press is now a
                    // no-op apart from arming the double-press window. The
                    // second press lands above. This is a small UX shift
                    // from "Ctrl+C quits instantly" but matches the hosted
                    // semantics and prevents accidental loss of unsubmitted
                    // input.
                }
                input::Action::SubmitInput(text) => {
                    // Push into input history (de-duped: skip if same as most recent).
                    if input_history.last().map(|s| s.as_str()) != Some(text.as_str()) {
                        input_history.push(text.clone());
                        // Cap to prevent unbounded growth in long sessions.
                        if input_history.len() > 200 {
                            input_history.remove(0);
                        }
                    }
                    input_history_index = None;
                    if let Some(tx) = outbound_tx.as_ref() {
                        // Bidirectional mode: emit UserInputSubmitted to the
                        // host. The host typically responds by sending back a
                        // UserMessageCreated which lands on stdin.
                        let ev = Event {
                            id: Uuid::new_v4(),
                            session_id,
                            seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                            timestamp_ms: now_ms(),
                            schema_version: SCHEMA_VERSION,
                            event_type: EventType::UserInputSubmitted,
                            payload: serde_json::json!({"text": text}),
                        };
                        if crate::scrolllog::event_enabled() {
                            crate::scrolllog::log_event(serde_json::json!({
                                "dir": "out",
                                "event_type": "UserInputSubmitted",
                                "seq": ev.seq,
                                "text_len": text.len(),
                                "text_preview": text.chars().take(80).collect::<String>(),
                            }));
                        }
                        let _ = tx.send(OutgoingEvent(ev)).await;
                    } else {
                        // Standalone mode: synthesise a local UserMessageCreated
                        // so the user sees their message in the transcript.
                        let ev = Event {
                            id: Uuid::new_v4(),
                            session_id,
                            seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                            timestamp_ms: now_ms(),
                            schema_version: SCHEMA_VERSION,
                            event_type: EventType::UserMessageCreated,
                            payload: serde_json::json!({"text": text}),
                        };
                        let _ = persist_tx.send(ev).await;
                    }
                }
                input::Action::ScrollUp(n) => {
                    let before = viewport.scroll_offset;
                    let before_frozen = viewport.frozen_visual_bottom;
                    viewport.scroll_up(n as i64);
                    crate::scrolllog::log_json(serde_json::json!({
                        "ev": "scroll_up",
                        "n": n,
                        "last_total_visual": viewport.last_total_visual,
                        "viewport_h": viewport.viewport_height,
                        "offset_before": before,
                        "offset_after": viewport.scroll_offset,
                        "frozen_before": before_frozen,
                        "frozen_after": viewport.frozen_visual_bottom,
                        "auto_follow": viewport.auto_follow,
                    }));
                    model.mark_dirty();
                }
                input::Action::ScrollDown(n) => {
                    let before = viewport.scroll_offset;
                    viewport.scroll_down(n as i64);
                    crate::scrolllog::log_json(serde_json::json!({
                        "ev": "scroll_down",
                        "n": n,
                        "viewport_h": viewport.viewport_height,
                        "offset_before": before,
                        "offset_after": viewport.scroll_offset,
                        "frozen_after": viewport.frozen_visual_bottom,
                        "auto_follow": viewport.auto_follow,
                    }));
                    model.mark_dirty();
                }
                input::Action::JumpToLatest => {
                    crate::scrolllog::log_json(serde_json::json!({
                        "ev": "jump_to_latest",
                        "offset_before": viewport.scroll_offset,
                    }));
                    viewport.jump_to_latest();
                    model.clear_history();
                    model.mark_dirty();
                }
                input::Action::CancelStream => {
                    esc_log("Action::CancelStream → emitting CancelRequested");
                    // First: give the user a visible local dismiss for the
                    // pinned splash. Without this, Esc at idle (no in-flight
                    // turn) looks like a no-op and the user thinks Esc is
                    // broken. We do this BEFORE the host emit so the dismiss
                    // is instant even if the pipe is slow.
                    let cleared_splash = model.clear_splash();
                    if cleared_splash {
                        esc_log("local dismiss: splash=true");
                    }
                    // Esc → interrupt. Same semantics as Ctrl+C: deny any
                    // open permission modal and emit CancelRequested so the
                    // host calls controller.abort() on the in-flight agent
                    // turn, then apply an optimistic local cancel. Shared
                    // with the top-of-loop in-flight pre-empt and Ctrl+C via
                    // `emit_cancel_requested`.
                    emit_cancel_requested(
                        &outbound_tx,
                        &mut model,
                        session_id,
                        &seq_counter,
                        &mut permission_feedback,
                    )
                    .await;
                    status = "canceled".into();
                    model.mark_dirty();
                }
                input::Action::Replay => {
                    model = RenderModel::new();
                    let evs = store.load_session(session_id).unwrap_or_default();
                    for ev in &evs { model.apply(ev); }
                    status = format!("replayed {} events", evs.len());
                    model.mark_dirty();
                }
                input::Action::PermissionSelectPrev => {
                    if let Some(pp) = model.pending_permission_mut() {
                        if pp.help_open { pp.help_open = false; }
                        else if pp.selected > 0 { pp.selected -= 1; }
                        else { pp.selected = 2; }
                        model.mark_dirty();
                    }
                }
                input::Action::PermissionSelectNext => {
                    if let Some(pp) = model.pending_permission_mut() {
                        if pp.help_open { pp.help_open = false; }
                        else { pp.selected = (pp.selected + 1) % 3; }
                        model.mark_dirty();
                    }
                }
                input::Action::PermissionToggleHelp => {
                    if let Some(pp) = model.pending_permission_mut() {
                        pp.help_open = !pp.help_open;
                        model.mark_dirty();
                    }
                }
                input::Action::PermissionConfirmSelected => {
                    // Translate the selected row into the same Action
                    // the digit keys would produce, then re-dispatch in
                    // the next loop iteration via fall-through.
                    if let Some(pp) = model.pending_permission() {
                        if pp.help_open {
                            if let Some(ppm) = model.pending_permission_mut() {
                                ppm.help_open = false;
                            }
                            model.mark_dirty();
                            continue;
                        }
                        action = match pp.selected {
                            0 => input::Action::PermissionAllowOnce,
                            1 => input::Action::PermissionAllowSession,
                            _ => input::Action::PermissionDeny,
                        };
                    } else {
                        continue;
                    }
                    // Fall through to PermissionAllowOnce|Session|Deny arm below.
                    // (Rust match doesn't allow C-style fallthrough — emulate
                    // by re-matching on the rewritten action via a nested if.)
                    if matches!(action,
                        input::Action::PermissionAllowOnce |
                        input::Action::PermissionAllowSession |
                        input::Action::PermissionDeny
                    ) {
                        // Inline the dispatcher to avoid Rust's no-fallthrough.
                        let (choice_str, fallback_kind) = match action {
                            input::Action::PermissionAllowOnce => ("allow_once", EventType::PermissionGranted),
                            input::Action::PermissionAllowSession => ("allow_session", EventType::PermissionGranted),
                            _ => ("deny", EventType::PermissionDenied),
                        };
                        let request_id = model.pending_permission().map(|p| p.request_id.clone()).unwrap_or_default();
                        if let Some(tx) = outbound_tx.as_ref() {
                            let mut payload = serde_json::json!({ "request_id": request_id, "choice": choice_str });
                            if !permission_feedback.trim().is_empty() {
                                payload["feedback"] = serde_json::Value::String(permission_feedback.trim().to_string());
                            }
                            let ev = Event {
                                id: Uuid::new_v4(), session_id,
                                seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                                timestamp_ms: now_ms(), schema_version: SCHEMA_VERSION,
                                event_type: EventType::PermissionResponse, payload,
                            };
                            let _ = tx.send(OutgoingEvent(ev)).await;
                        } else {
                            let ev = Event {
                                id: Uuid::new_v4(), session_id,
                                seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                                timestamp_ms: now_ms(), schema_version: SCHEMA_VERSION,
                                event_type: fallback_kind,
                                payload: serde_json::json!({"request_id": request_id}),
                            };
                            let _ = persist_tx.send(ev).await;
                        }
                        model.clear_pending_permission();
                        permission_feedback.clear();
                        model.mark_dirty();
                    }
                }
                input::Action::PermissionAllowOnce
                | input::Action::PermissionAllowSession
                | input::Action::PermissionDeny => {
                    let (choice_str, fallback_kind) = match action {
                        input::Action::PermissionAllowOnce => ("allow_once", EventType::PermissionGranted),
                        input::Action::PermissionAllowSession => ("allow_session", EventType::PermissionGranted),
                        _ => ("deny", EventType::PermissionDenied),
                    };
                    let request_id = model
                        .pending_permission()
                        .map(|p| p.request_id.clone())
                        .unwrap_or_default();
                    if let Some(tx) = outbound_tx.as_ref() {
                        let mut payload = serde_json::json!({
                            "request_id": request_id,
                            "choice": choice_str,
                        });
                        if !permission_feedback.trim().is_empty() {
                            payload["feedback"] =
                                serde_json::Value::String(permission_feedback.trim().to_string());
                        }
                        let ev = Event {
                            id: Uuid::new_v4(),
                            session_id,
                            seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                            timestamp_ms: now_ms(),
                            schema_version: SCHEMA_VERSION,
                            event_type: EventType::PermissionResponse,
                            payload,
                        };
                        let _ = tx.send(OutgoingEvent(ev)).await;
                    } else {
                        // Standalone mode: synthesise the resulting
                        // Granted/Denied locally so the user sees feedback.
                        let ev = Event {
                            id: Uuid::new_v4(),
                            session_id,
                            seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                            timestamp_ms: now_ms(),
                            schema_version: SCHEMA_VERSION,
                            event_type: fallback_kind,
                            payload: serde_json::json!({"request_id": request_id}),
                        };
                        let _ = persist_tx.send(ev).await;
                    }
                    model.clear_pending_permission();
                    permission_feedback.clear();
                    model.mark_dirty();
                }
                input::Action::ReplayTogglePlay => {
                    if let Some(rs) = replay_state.as_mut() {
                        rs.playing = !rs.playing;
                        rs.last_tick = tokio::time::Instant::now();
                        rs.accumulator = 0.0;
                        status = replay_status(rs);
                        model.mark_dirty();
                    }
                }
                input::Action::ReplayStepForward => {
                    if let Some(rs) = replay_state.as_mut() {
                        rs.playing = false;
                        rs.step_forward(1, &mut model);
                        status = replay_status(rs);
                        model.mark_dirty();
                    }
                }
                input::Action::ReplayStepBackward => {
                    if let Some(rs) = replay_state.as_mut() {
                        rs.playing = false;
                        rs.step_backward(1, &mut model);
                        status = replay_status(rs);
                        model.mark_dirty();
                    }
                }
                input::Action::ReplayFaster => {
                    if let Some(rs) = replay_state.as_mut() {
                        rs.bump_speed(true);
                        status = replay_status(rs);
                        model.mark_dirty();
                    }
                }
                input::Action::ReplaySlower => {
                    if let Some(rs) = replay_state.as_mut() {
                        rs.bump_speed(false);
                        status = replay_status(rs);
                        model.mark_dirty();
                    }
                }
                input::Action::ReplayRestart => {
                    if let Some(rs) = replay_state.as_mut() {
                        rs.step_backward(rs.position, &mut model);
                        rs.playing = false;
                        status = replay_status(rs);
                        model.mark_dirty();
                    }
                }
                input::Action::ReplayJumpPct(pct) => {
                    if let Some(rs) = replay_state.as_mut() {
                        let total = rs.events.len();
                        let target = (total as u64 * pct as u64 / 100) as usize;
                        if target >= rs.position {
                            let n = target - rs.position;
                            rs.step_forward(n, &mut model);
                        } else {
                            let n = rs.position - target;
                            rs.step_backward(n, &mut model);
                        }
                        rs.playing = false;
                        status = replay_status(rs);
                        model.mark_dirty();
                    }
                }
                input::Action::ReplayJumpEnd => {
                    if let Some(rs) = replay_state.as_mut() {
                        let remaining = rs.events.len().saturating_sub(rs.position);
                        rs.step_forward(remaining, &mut model);
                        rs.playing = false;
                        status = replay_status(rs);
                        model.mark_dirty();
                    }
                }
                input::Action::ToggleInspector => {
                    inspector_open = !inspector_open;
                    if inspector_open {
                        inspector_cursor = 0;
                        inspector_cached_seq = None;
                    }
                    model.mark_dirty();
                }
                input::Action::ToggleHelp => {
                    help_open = !help_open;
                    model.mark_dirty();
                }
                input::Action::ToggleMetrics => {
                    metrics_open = !metrics_open;
                    model.mark_dirty();
                }
                input::Action::CycleTheme => {
                    theme_name = camouflage_renderer::theme::Theme::next_after(&theme_name).to_string();
                    status = format!("theme: {theme_name}");
                    model.mark_dirty();
                }
                input::Action::ToggleToolOutput => {
                    tool_output_open = !tool_output_open;
                    model.mark_dirty();
                }
                input::Action::ToggleMouseCapture => {
                    // Write the SGR mouse enable/disable sequences directly
                    // to stdout so the terminal flips reporting on next
                    // poll. We keep bracketed paste alone (handled
                    // separately at setup/teardown).
                    use std::io::Write;
                    mouse_capture_on = !mouse_capture_on;
                    let seq: &[u8] = if mouse_capture_on {
                        b"\x1b[?1000h\x1b[?1006h"
                    } else {
                        b"\x1b[?1006l\x1b[?1000l"
                    };
                    let _ = terminal.backend_mut().write_all(seq);
                    let _ = terminal.backend_mut().flush();
                    status = if mouse_capture_on {
                        "mouse capture ON (wheel scrolls)".into()
                    } else {
                        "mouse capture OFF (drag to select; press Shift+S to re-enable)".into()
                    };
                    model.mark_dirty();
                }
                input::Action::InspectorCursorUp => {
                    if inspector_open {
                        inspector_cursor = inspector_cursor.saturating_add(1)
                            .min(model.combined_len().saturating_sub(1));
                        model.mark_dirty();
                    }
                }
                input::Action::InspectorCursorDown => {
                    if inspector_open {
                        inspector_cursor = inspector_cursor.saturating_sub(1);
                        model.mark_dirty();
                    }
                }
                input::Action::CycleFilter => {
                    row_filter = row_filter.next();
                    status = format!("filter: {}", row_filter.label());
                    model.mark_dirty();
                }
                input::Action::SearchOpen => {
                    search_open = true;
                    search_query.clear();
                    model.mark_dirty();
                }
                input::Action::SearchClose => {
                    search_open = false;
                    model.mark_dirty();
                }
                input::Action::SearchChar(c) => {
                    if search_open {
                        search_query.push(c);
                        model.mark_dirty();
                    }
                }
                input::Action::SearchBackspace => {
                    if search_open {
                        search_query.pop();
                        model.mark_dirty();
                    }
                }
                input::Action::SearchSubmit => {
                    if search_open {
                        search_matches = run_search(&model, &search_query);
                        search_current = 0;
                        search_open = false;
                        if !search_matches.is_empty() {
                            scroll_to_seq(&mut viewport, &model, search_matches[0]);
                            status = format!(
                                "search '{}': 1/{}",
                                search_query,
                                search_matches.len()
                            );
                        } else {
                            status = format!("search '{}': no matches", search_query);
                        }
                        model.mark_dirty();
                    }
                }
                input::Action::SearchNext => {
                    if !search_matches.is_empty() {
                        search_current = (search_current + 1) % search_matches.len();
                        scroll_to_seq(&mut viewport, &model, search_matches[search_current]);
                        status = format!(
                            "search '{}': {}/{}",
                            search_query,
                            search_current + 1,
                            search_matches.len()
                        );
                        model.mark_dirty();
                    }
                }
                input::Action::SearchPrev => {
                    if !search_matches.is_empty() {
                        search_current = if search_current == 0 {
                            search_matches.len() - 1
                        } else {
                            search_current - 1
                        };
                        scroll_to_seq(&mut viewport, &model, search_matches[search_current]);
                        status = format!(
                            "search '{}': {}/{}",
                            search_query,
                            search_current + 1,
                            search_matches.len()
                        );
                        model.mark_dirty();
                    }
                }
                input::Action::BookmarkAdd => {
                    // Bookmark the row currently at the bottom of the visible
                    // window (or the inspector cursor if it's open).
                    let target = if inspector_open {
                        inspector_focused_seq(&model, inspector_cursor)
                    } else {
                        model.rows().back().map(|r| r.seq).or_else(|| {
                            model.history_rows().last().map(|r| r.seq)
                        })
                    };
                    if let Some(seq) = target {
                        if !bookmarks.contains(&seq) {
                            bookmarks.push(seq);
                            bookmarks.sort();
                        }
                        status = format!("bookmark added @ seq {} ({} total)", seq, bookmarks.len());
                        model.mark_dirty();
                    }
                }
                input::Action::BookmarkNext => {
                    if !bookmarks.is_empty() {
                        bookmark_cursor = (bookmark_cursor + 1) % bookmarks.len();
                        let seq = bookmarks[bookmark_cursor];
                        scroll_to_seq(&mut viewport, &model, seq);
                        status = format!(
                            "bookmark {}/{} @ seq {}",
                            bookmark_cursor + 1,
                            bookmarks.len(),
                            seq
                        );
                        model.mark_dirty();
                    }
                }
                input::Action::None => {
                    model.mark_dirty();
                }
            }
        }

        // Advance replay playback based on elapsed wall-clock time.
        if let Some(rs) = replay_state.as_mut() {
            if rs.playing && !rs.is_complete() {
                let now = tokio::time::Instant::now();
                let dt = now.duration_since(rs.last_tick).as_secs_f32();
                rs.last_tick = now;
                rs.accumulator += dt * rs.speed_eps * rs.speed_mult;
                let to_apply = rs.accumulator.floor() as usize;
                if to_apply > 0 {
                    rs.accumulator -= to_apply as f32;
                    let applied = rs.step_forward(to_apply, &mut model);
                    if applied > 0 {
                        status = replay_status(rs);
                    }
                    if rs.is_complete() {
                        rs.playing = false;
                        status = replay_status(rs);
                    }
                }
            } else {
                rs.last_tick = tokio::time::Instant::now();
            }
        }

        // Maybe request more history. Trigger when the user is scrolled
        // near the top of what's in memory AND the model's earliest visible
        // seq is > 0 (i.e. older events exist in the store and we haven't
        // paged them in yet).
        if !history_inflight {
            if let Some(earliest_visible) = model.earliest_visible_seq() {
                if earliest_visible > 0 {
                    // "Near the top" in the visual-line scroll model: the
                    // user has scrolled most of the way up the frozen
                    // window (within viewport_h of the top).
                    let near_top = viewport.frozen_visual_bottom.is_some()
                        && (viewport.scroll_offset
                            >= viewport.last_total_visual
                                - viewport.viewport_height as i64
                                - 8);
                    if near_top {
                        // Cap the lower bound at the post-/clear floor so we
                        // don't fetch (and the model wouldn't accept) events
                        // older than the most recent TranscriptCleared.
                        let floor = model.history_floor_seq();
                        let to = earliest_visible;
                        let from = (to - HISTORY_CHUNK).max(floor);
                        if from < to {
                            tracing::info!(from, to, "history fetch requested");
                            let _ = history_req_tx
                                .send(HistoryReq { session: session_id, from_seq: from, to_seq: to })
                                .await;
                            history_inflight = true;
                        }
                    }
                }
            }
        }

        tokio::select! {
            biased;
            Some(resp) = history_done_rx.recv() => {
                let n = resp.rows.len();
                tracing::info!(from_seq = resp.from_seq, rows = n, "history fetch returned");
                if !resp.rows.is_empty() {
                    model.prepend_history(resp.rows);
                }
                history_inflight = false;
                model.mark_dirty();
            }
            Some(ev) = rendered_rx.recv() => {
                crash_ring_push(&ev);
                if crate::scrolllog::event_enabled() {
                    crate::scrolllog::log_event(serde_json::json!({
                        "dir": "in",
                        "event_type": format!("{:?}", ev.event_type),
                        "seq": ev.seq,
                        "payload_preview": ev.payload.to_string().chars().take(160).collect::<String>(),
                    }));
                }
                model.apply(&ev);
                total_events += 1;
                events_since_window += 1;
                status = match ev.event_type {
                    EventType::AssistantTokenDelta | EventType::AssistantStreamStarted => "streaming".into(),
                    EventType::AssistantMessageCompleted => "idle".into(),
                    EventType::ToolExecutionStarted | EventType::ToolExecutionStdout | EventType::ToolExecutionStderr => "tool".into(),
                    EventType::ToolExecutionFinished => "idle".into(),
                    EventType::RuntimeError => "error".into(),
                    _ => status,
                };
                let mut drained = 0;
                while drained < 1024 {
                    match rendered_rx.try_recv() {
                        Ok(ev2) => {
                            crash_ring_push(&ev2);
                            if crate::scrolllog::event_enabled() {
                                crate::scrolllog::log_event(serde_json::json!({
                                    "dir": "in",
                                    "event_type": format!("{:?}", ev2.event_type),
                                    "seq": ev2.seq,
                                    "payload_preview": ev2.payload.to_string().chars().take(160).collect::<String>(),
                                }));
                            }
                            model.apply(&ev2);
                            total_events += 1;
                            events_since_window += 1;
                            drained += 1;
                        }
                        Err(_) => break,
                    }
                }
            }
            _ = ticker.tick() => {
                frame_counter = frame_counter.wrapping_add(1);
                // Drop finished background tasks once the celebration
                // window has elapsed. Matches Ink's TaskList ~1.5s glow.
                let before = model.background_tasks().len();
                model.sweep_finished_tasks(now_ms());
                if model.background_tasks().len() != before {
                    model.mark_dirty();
                }
                // Expire transient toasts on the same tick so they vanish on
                // time even when nothing else is happening on screen.
                if model.sweep_expired_toasts(now_ms()) {
                    model.mark_dirty();
                }
                // Refresh inspector cache if the cursor moved or the row at
                // the cursor has changed seq (e.g. due to new events).
                if inspector_open {
                    if let Some(focused_seq) = inspector_focused_seq(&model, inspector_cursor) {
                        if inspector_cached_seq != Some(focused_seq) {
                            if let Ok(events) =
                                store.load_range(session_id, focused_seq, focused_seq + 1)
                            {
                                if let Some(ev) = events.into_iter().next() {
                                    inspector_cached_json = serde_json::to_string_pretty(&ev)
                                        .unwrap_or_else(|_| "<json error>".into());
                                    inspector_cached_seq = Some(focused_seq);
                                }
                            }
                        }
                    } else {
                        inspector_cached_seq = None;
                        inspector_cached_json.clear();
                    }
                }
                // A spinner is "alive" if any tool isn't finished OR the host's
                // phase segment is in a spinnable state OR any todo is in
                // progress (so the elapsed-time ticker updates). When alive,
                // redraw every tick so glyphs rotate and timers tick even if
                // the model is clean.
                let any_unfinished_tool = model.tools().values().any(|t| !t.finished);
                let phase_spins = model
                    .status_segments()
                    .get("phase")
                    .map(|p| matches!(p.as_str(), "thinking" | "streaming" | "tool" | "running"))
                    .unwrap_or(false);
                let any_in_progress_todo = model.todos().iter().any(|t| matches!(t.status, camouflage_renderer::model::TodoStatus::InProgress));
                let spinner_alive = any_unfinished_tool || phase_spins || any_in_progress_todo;
                if model.dirty() || spinner_alive {
                    let insp = if inspector_open {
                        Some(draw::InspectorView {
                            cursor_offset: inspector_cursor,
                            focused_seq: inspector_cached_seq,
                            json: &inspector_cached_json,
                        })
                    } else {
                        None
                    };
                    let filter = match row_filter {
                        RowFilter::All => None,
                        RowFilter::Errors => Some(draw::RowFilterKind::Errors),
                        RowFilter::Tools => Some(draw::RowFilterKind::Tools),
                        RowFilter::Patches => Some(draw::RowFilterKind::Patches),
                        RowFilter::Permissions => Some(draw::RowFilterKind::Permissions),
                    };
                    let search_view = if search_open {
                        Some(draw::SearchView { query: &search_query })
                    } else {
                        None
                    };
                    // Update 1-second rate window for the metrics overlay.
                    let now = std::time::Instant::now();
                    if now.duration_since(last_rate_window) >= std::time::Duration::from_secs(1) {
                        let secs = now.duration_since(last_rate_window).as_secs_f64().max(0.001);
                        events_per_sec = events_since_window as f64 / secs;
                        events_since_window = 0;
                        last_rate_window = now;
                    }
                    let metrics = if metrics_open {
                        Some(draw::MetricsView {
                            total_events,
                            events_per_sec,
                            frame_us: last_frame_time_us,
                            session_secs: session_started_at.elapsed().as_secs(),
                            rows_live: model.rows().len(),
                            row_cap: cfg.row_cap.unwrap_or(camouflage_renderer::model::DEFAULT_ROW_CAP),
                            history_rows: model.history_rows().len(),
                            background_tasks: model.background_tasks().len(),
                        })
                    } else {
                        None
                    };
                    let draw_start = std::time::Instant::now();
                    let theme = camouflage_renderer::theme::Theme::builtin(&theme_name)
                        .unwrap_or_else(|| {
                            camouflage_renderer::theme::Theme::builtin("default-dark").unwrap()
                        });
                    let replay_view = replay_state.as_ref().map(|rs| draw::ReplayView {
                        position: rs.position,
                        total: rs.events.len(),
                        playing: rs.playing,
                        speed_mult: rs.speed_mult,
                    });
                    let now_ms = now_ms();
                    draw::render(
                        &mut terminal,
                        &model,
                        &mut viewport,
                        &input_buf,
                        input_cursor,
                        &status,
                        frame_counter,
                        insp,
                        filter,
                        search_view,
                        help_open,
                        metrics,
                        replay_view,
                        theme,
                        tool_output_open,
                        &permission_feedback,
                        slash_picker_index,
                        mention_picker_index,
                        slash_sub_state.as_ref(),
                        &mut todo_scroll_offset,
                        now_ms,
                        paste_preview.as_deref(),
                    )?;
                    last_frame_time_us = draw_start.elapsed().as_micros();
                    model.mark_clean();
                }
                // Resize check (size() is cheap; no kqueue needed)
                if let Ok((w, h)) = crossterm::terminal::size() {
                    // header(1) + status(1) + input(3) + optional task ribbon(1) = 5–6 reserved.
                    let reserved = 5 + if model.background_tasks().is_empty() { 0 } else { 1 };
                    let visible = h.saturating_sub(reserved);
                    if w != viewport.viewport_width || visible != viewport.viewport_height {
                        viewport.resize(visible, w);
                        model.mark_dirty();
                    }
                }
            }
        }
    }
}

fn shutdown(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    result: Result<()>,
) -> Result<()> {
    teardown_terminal(terminal).ok();
    result
}

/// Subset of row kinds the user wants to see. Cycled by pressing `f`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowFilter {
    All,
    Errors,
    Tools,
    Patches,
    Permissions,
}

impl RowFilter {
    fn next(self) -> Self {
        match self {
            Self::All => Self::Errors,
            Self::Errors => Self::Tools,
            Self::Tools => Self::Patches,
            Self::Patches => Self::Permissions,
            Self::Permissions => Self::All,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Errors => "errors",
            Self::Tools => "tools",
            Self::Patches => "patches",
            Self::Permissions => "permissions",
        }
    }
}

/// Substring search across all currently-loaded rows. Returns the seqs of
/// matching rows in order from oldest to newest. Case-insensitive.
fn run_search(model: &RenderModel, query: &str) -> Vec<i64> {
    if query.is_empty() {
        return Vec::new();
    }
    let q = query.to_lowercase();
    let mut hits = Vec::new();
    for r in model.history_rows().iter().chain(model.rows().iter()) {
        if r.text.to_lowercase().contains(&q) {
            hits.push(r.seq);
        }
    }
    hits
}

/// Adjust the viewport scroll so the row matching `seq` is visible.
/// scroll_offset is in visual lines now, but we don't have width info
/// here — so we approximate row-offset-from-bottom as a visual-line
/// offset (each row contributes >= 1 line). Close enough to put the
/// target row inside the viewport for search-jump.
fn scroll_to_seq(viewport: &mut ViewportState, model: &RenderModel, seq: i64) {
    let mut idx_from_bottom: Option<i64> = None;
    let mut cnt: i64 = 0;
    for r in model.history_rows().iter().chain(model.rows().iter()).rev() {
        if r.seq == seq {
            idx_from_bottom = Some(cnt);
            break;
        }
        cnt += 1;
    }
    if let Some(offset) = idx_from_bottom {
        if offset == 0 {
            viewport.jump_to_latest();
        } else {
            viewport.frozen_visual_bottom = Some(viewport.last_total_visual);
            viewport.scroll_offset = (offset - viewport.viewport_height as i64 / 2).max(0);
            viewport.auto_follow = false;
        }
    }
}

/// Resolve the seq of the row currently under the inspector cursor.
/// `cursor_offset == 0` means the bottom-most (newest) visible row.
fn inspector_focused_seq(model: &RenderModel, cursor_offset: usize) -> Option<i64> {
    let total = model.combined_len();
    if total == 0 {
        return None;
    }
    let idx = total.saturating_sub(1).saturating_sub(cursor_offset);
    let history = model.history_rows();
    if idx < history.len() {
        history.get(idx).map(|r| r.seq)
    } else {
        let live_idx = idx - history.len();
        model.rows().get(live_idx).map(|r| r.seq)
    }
}

fn replay_status(rs: &ReplayState) -> String {
    let state = if rs.is_complete() {
        "end"
    } else if rs.playing {
        "play"
    } else {
        "pause"
    };
    format!(
        "replay {}/{} @ {:.2}x [{}]",
        rs.position,
        rs.events.len(),
        rs.speed_mult,
        state
    )
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    // Enable SGR mouse mode: basic press/release (1000) + SGR extension
    // (1006) for coordinates > 223 and uniform parsing. With these on,
    // the terminal sends ESC [ < button ; x ; y M|m for mouse events
    // (including wheel as buttons 64/65) instead of translating the
    // scroll wheel into Up/Down arrow keys — which was polluting the
    // input-history navigation. Wired into the /dev/tty reader's parser.
    //
    // Also enable bracketed-paste mode (?2004): the terminal wraps any
    // pasted text in ESC [200~ … ESC [201~, which the parser turns into
    // a single Key::Paste event. Without this, pasting a block that
    // contains a newline would submit the partial input the moment the
    // newline arrived (since 0x0a is Enter in raw mode).
    use std::io::Write;
    let _ = stdout.write_all(b"\x1b[?1000h\x1b[?1006h\x1b[?2004h");
    let _ = stdout.flush();
    let backend = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;
    term.clear()?;
    Ok(term)
}

fn teardown_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let _ = disable_raw_mode();
    // Disable mouse capture and bracketed paste (must come before
    // LeaveAlternateScreen so the sequence isn't lost when we drop the
    // alt screen).
    use std::io::Write;
    let _ = terminal.backend_mut().write_all(b"\x1b[?2004l\x1b[?1006l\x1b[?1000l");
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = std::io::stdout().execute(LeaveAlternateScreen);
        // Crash-replay dump: if the shared ring buffer has been wired up,
        // flush its contents to crash-<unix_ts>.ndjson in cwd so the user
        // can attach a reproducer to a bug report.
        if let Some(ring) = CRASH_RING.get() {
            if let Ok(buf) = ring.lock() {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let path = std::path::PathBuf::from(format!("crash-{ts}.ndjson"));
                let mut header = format!("# camouflage crash dump\n# panic: {info}\n# events: {}\n", buf.len());
                if let Some(ev) = buf.front() {
                    header.push_str(&format!("# session: {}\n", ev.session_id));
                }
                let mut body = String::with_capacity(64 * buf.len());
                for ev in buf.iter() {
                    if let Ok(s) = serde_json::to_string(ev) {
                        body.push_str(&s);
                        body.push('\n');
                    }
                }
                let combined = format!("{header}{body}");
                let _ = std::fs::write(&path, combined);
                eprintln!("camouflage: crash dump written to {}", path.display());
            }
        }
        prev(info);
    }));
}

/// Shared ring buffer of recent events for crash-replay. Populated by the
/// event-receive hot path; flushed to disk by the panic hook.
pub(crate) static CRASH_RING: std::sync::OnceLock<
    std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<camouflage_protocol::Event>>>,
> = std::sync::OnceLock::new();

pub(crate) const CRASH_RING_CAP: usize = 256;

pub(crate) fn crash_ring_push(ev: &camouflage_protocol::Event) {
    if let Some(ring) = CRASH_RING.get() {
        if let Ok(mut buf) = ring.lock() {
            if buf.len() >= CRASH_RING_CAP {
                buf.pop_front();
            }
            buf.push_back(ev.clone());
        }
    }
}

/// CC-4 — record a wizard step result, advance, and either emit
/// WizardCompleted (with accumulated results) or let the next step's
/// sub-modal take over.
async fn advance_wizard_after(
    outbound_tx: &Option<mpsc::Sender<OutgoingEvent>>,
    model: &mut RenderModel,
    session_id: Uuid,
    seq_counter: &Arc<AtomicI64>,
    result: camouflage_renderer::model::WizardStepResult,
) {
    use camouflage_renderer::model::WizardAdvance;
    let wizard_id = model.active_wizard().map(|w| w.id.clone());
    let advance = model.wizard_advance(result);
    if let (Some(wizard_id), Some(WizardAdvance::Completed(results))) = (wizard_id, advance) {
        // Serialize results: each step id → JSON value (string / bool / object).
        let mut results_json = serde_json::Map::new();
        for (step_id, r) in results {
            use camouflage_renderer::model::WizardStepResult as R;
            let v = match r {
                R::Select(s) => serde_json::Value::String(s),
                R::Confirm(b) => serde_json::Value::Bool(b),
                R::Form(map) => {
                    let mut obj = serde_json::Map::new();
                    for (k, v) in map { obj.insert(k, serde_json::Value::String(v)); }
                    serde_json::Value::Object(obj)
                }
            };
            results_json.insert(step_id, v);
        }
        model.clear_wizard();
        if let Some(tx) = outbound_tx.as_ref() {
            let ev = Event {
                id: Uuid::new_v4(),
                session_id,
                seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                timestamp_ms: now_ms(),
                schema_version: SCHEMA_VERSION,
                event_type: EventType::WizardCompleted,
                payload: serde_json::json!({
                    "id": wizard_id,
                    "results": serde_json::Value::Object(results_json),
                }),
            };
            let _ = tx.send(OutgoingEvent(ev)).await;
        }
    }
    model.mark_dirty();
}

/// Loud env-gated log for tracing the Esc → CancelRequested chain. Set
/// CAMOUFLAGE_DEBUG_ESC=1 in the host env to enable; output lands on
/// stderr which the SDK inherits in renderToTerminal=true, so messages
/// show up in the user's terminal alongside host logs.
fn esc_log(s: &str) {
    if std::env::var_os("CAMOUFLAGE_DEBUG_ESC").is_some() {
        eprintln!("[camouflage:esc] {s}");
    }
}

/// Abort the running agent turn — the shared body behind both Esc and
/// Ctrl+C. If a permission modal is open, deny it first so the tool gets a
/// clean "rejected" response and the modal closes, then emit
/// `CancelRequested` so the host calls `controller.abort()` on the
/// in-flight turn. Finally apply an optimistic local cancel so the user
/// gets immediate visual feedback (spinners stop, status leaves
/// "thinking") even before the host echoes back terminal events.
async fn emit_cancel_requested(
    outbound_tx: &Option<mpsc::Sender<OutgoingEvent>>,
    model: &mut RenderModel,
    session_id: Uuid,
    seq_counter: &Arc<AtomicI64>,
    permission_feedback: &mut String,
) {
    if outbound_tx.is_none() {
        esc_log("WARN outbound_tx is None — no host pipe wired up");
    }
    if let Some(tx) = outbound_tx.as_ref() {
        if let Some(pp) = model.pending_permission() {
            let resp = Event {
                id: Uuid::new_v4(),
                session_id,
                seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                timestamp_ms: now_ms(),
                schema_version: SCHEMA_VERSION,
                event_type: EventType::PermissionResponse,
                payload: serde_json::json!({
                    "request_id": pp.request_id,
                    "choice": "deny",
                    "feedback": permission_feedback,
                }),
            };
            let _ = tx.send(OutgoingEvent(resp)).await;
            model.clear_pending_permission();
            permission_feedback.clear();
        }
        let ev = Event {
            id: Uuid::new_v4(),
            session_id,
            seq: seq_counter.fetch_add(1, Ordering::Relaxed),
            timestamp_ms: now_ms(),
            schema_version: SCHEMA_VERSION,
            event_type: EventType::CancelRequested,
            payload: serde_json::json!({}),
        };
        let send_res = tx.send(OutgoingEvent(ev)).await;
        esc_log(&format!(
            "CancelRequested send → {}",
            if send_res.is_ok() { "ok" } else { "ERR (receiver dropped)" }
        ));
    }
    model.local_cancel_all(now_ms());
}

/// CC-4 — user cancelled a wizard step. Emit WizardCancelled with the
/// current step index, then clear the wizard.
async fn emit_wizard_cancelled(
    outbound_tx: &Option<mpsc::Sender<OutgoingEvent>>,
    model: &mut RenderModel,
    session_id: Uuid,
    seq_counter: &Arc<AtomicI64>,
) {
    let (wizard_id, at_step) = match model.active_wizard() {
        Some(w) => (w.id.clone(), w.current),
        None => return,
    };
    model.clear_wizard();
    if let Some(tx) = outbound_tx.as_ref() {
        let ev = Event {
            id: Uuid::new_v4(),
            session_id,
            seq: seq_counter.fetch_add(1, Ordering::Relaxed),
            timestamp_ms: now_ms(),
            schema_version: SCHEMA_VERSION,
            event_type: EventType::WizardCancelled,
            payload: serde_json::json!({ "id": wizard_id, "at_step": at_step }),
        };
        let _ = tx.send(OutgoingEvent(ev)).await;
    }
    model.mark_dirty();
}

/// Returns the `@`-prefixed partial at the end of `buf`, if any.
/// "do X with @auth" → Some("auth")
/// "do X with @"     → Some("")
/// "do X @auth then" → None  (trailing space breaks the match)
/// "do X"            → None
fn last_mention_partial(buf: &str) -> Option<String> {
    let at = buf.rfind('@')?;
    let tail = &buf[at + 1..];
    if tail.contains(char::is_whitespace) {
        return None;
    }
    Some(tail.to_string())
}

/// Parse a slash command's `args_hint` into a set of discrete options
/// for the sub-picker. Returns `Some((options, optional))` only when
/// the hint is a pipe-separated list of bare words — e.g. `edit|plan|auto`
/// or `[on|off]`. Hints containing `<placeholder>` (free text required)
/// or nested brackets are rejected — they need real typing, not a list.
/// A trailing `...` marker (e.g. `[on|off|search ...]`) is stripped so
/// the leading discrete tokens still drive a picker.
pub(crate) fn parse_discrete_args(hint: &str) -> Option<(Vec<String>, bool)> {
    let trimmed = hint.trim();
    let (inner, optional) = if let Some(stripped) = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
    {
        (stripped, true)
    } else {
        (trimmed, false)
    };
    if inner.is_empty() {
        return None;
    }
    // Free-text placeholders and nested optional groups can't be picked.
    if inner.contains('<') || inner.contains('[') {
        return None;
    }
    let inner = inner.trim_end_matches("...").trim();
    let opts: Vec<String> = inner
        .split('|')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if opts.is_empty() {
        return None;
    }
    Some((opts, optional))
}

#[cfg(test)]
mod arg_parser_tests {
    use super::parse_discrete_args;
    #[test]
    fn bare_pipes() {
        assert_eq!(
            parse_discrete_args("edit|plan|auto"),
            Some((vec!["edit".into(), "plan".into(), "auto".into()], false))
        );
    }
    #[test]
    fn optional_bracketed() {
        assert_eq!(
            parse_discrete_args("[on|off]"),
            Some((vec!["on".into(), "off".into()], true))
        );
    }
    #[test]
    fn trailing_dots() {
        assert_eq!(
            parse_discrete_args("[on|off|clear|search ...]"),
            Some((vec!["on".into(), "off".into(), "clear".into(), "search".into()], true))
        );
    }
    #[test]
    fn rejects_placeholders() {
        assert!(parse_discrete_args("<prompt>").is_none());
        assert!(parse_discrete_args("[<name>]").is_none());
        assert!(parse_discrete_args("[send] [note]").is_none());
    }
}
