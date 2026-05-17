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
    pub fps: u32,
    pub row_cap: Option<usize>,
    pub emit_responses: bool,
    /// Optional pre-opened file descriptor for outbound NDJSON. When set,
    /// takes precedence over `emit_responses`. Host wires this up with
    /// e.g. `child_process.spawn(cmd, { stdio: [...,"pipe"] })` and points
    /// the renderer at the resulting fd number.
    pub responses_fd: Option<i32>,
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
        *model = RenderModel::new();
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
    // v0.4.5: @-mention picker selection cursor.
    let mut mention_picker_index: usize = 0;
    // v0.4.6: per-session input history. Each submitted user prompt is
    // pushed; Up/Down at the input prompt walks through them. `index` is
    // the cursor (None = "live" buffer, not browsing history).
    let mut input_history: Vec<String> = Vec::new();
    let mut input_history_index: Option<usize> = None;

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
                        crate::tty::Key::Up => {
                            slash_picker_index = slash_picker_index.saturating_sub(1);
                            model.mark_dirty();
                            continue;
                        }
                        crate::tty::Key::Down => {
                            slash_picker_index = (slash_picker_index + 1).min(matches.len() - 1);
                            model.mark_dirty();
                            continue;
                        }
                        crate::tty::Key::Enter => {
                            let idx = slash_picker_index.min(matches.len() - 1);
                            input_buf = format!("/{} ", matches[idx].name);
                            slash_picker_index = 0;
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
            // Doesn't interfere with the existing "scroll transcript" use
            // of Up/Down because that's bound when input_buf is empty —
            // but if there's history, history takes priority.
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
                && !tool_output_open;
            if input_focused && !input_history.is_empty() {
                match key {
                    crate::tty::Key::Up => {
                        let next = match input_history_index {
                            None => input_history.len() - 1,
                            Some(i) => i.saturating_sub(1),
                        };
                        input_history_index = Some(next);
                        input_buf = input_history[next].clone();
                        model.mark_dirty();
                        continue;
                    }
                    crate::tty::Key::Down => {
                        match input_history_index {
                            Some(i) if i + 1 < input_history.len() => {
                                input_history_index = Some(i + 1);
                                input_buf = input_history[i + 1].clone();
                            }
                            Some(_) => {
                                // Past the last entry → back to a fresh buffer.
                                input_history_index = None;
                                input_buf.clear();
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
            if matches!(key, crate::tty::Key::Esc) {
                if help_open || metrics_open || tool_output_open {
                    help_open = false;
                    metrics_open = false;
                    tool_output_open = false;
                    model.mark_dirty();
                    continue;
                }
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
                            // Move within the filtered list. selected may not
                            // be in `visible`, in which case treat as 0.
                            let pos = visible.iter().position(|&i| i == sl.selected).unwrap_or(0);
                            let next = pos.saturating_sub(1);
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
                            let next = (pos + 1).min(visible.len().saturating_sub(1));
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
                            mention_picker_index = mention_picker_index.saturating_sub(1);
                            model.mark_dirty();
                            continue;
                        }
                        crate::tty::Key::Down => {
                            mention_picker_index = (mention_picker_index + 1).min(matches.len() - 1);
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
            // Priority order: search prompt → pending permission widget →
            // inspector cursor (if open) → replay controls (if --replay) →
            // normal handler.
            let action = if search_open {
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
                (replay_state.as_ref(), input::handle_key_replay(key, &input_buf))
            {
                act
            } else {
                input::handle_key(key, &mut input_buf)
            };
            match action {
                input::Action::Quit => return shutdown(&mut terminal, Ok(())),
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
                    viewport.scroll_up(n as i64, model.combined_len() as i64);
                    model.mark_dirty();
                }
                input::Action::ScrollDown(n) => {
                    viewport.scroll_down(n as i64);
                    model.mark_dirty();
                }
                input::Action::JumpToLatest => {
                    viewport.jump_to_latest();
                    model.clear_history();
                    model.mark_dirty();
                }
                input::Action::CancelStream => {
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
                    let near_top = (viewport.scroll_offset + viewport.viewport_height as i64)
                        >= model.combined_len() as i64 - 8;
                    if near_top {
                        let to = earliest_visible;
                        let from = (to - HISTORY_CHUNK).max(0);
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
                // phase segment is in a spinnable state. When alive, redraw
                // every tick so the glyph rotates even if the model is clean.
                let any_unfinished_tool = model.tools().values().any(|t| !t.finished);
                let phase_spins = model
                    .status_segments()
                    .get("phase")
                    .map(|p| matches!(p.as_str(), "thinking" | "streaming" | "tool" | "running"))
                    .unwrap_or(false);
                let spinner_alive = any_unfinished_tool || phase_spins;
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
                    draw::render(
                        &mut terminal,
                        &model,
                        &viewport,
                        &input_buf,
                        &status,
                        frame_counter,
                        insp,
                        filter,
                        search_view,
                        help_open,
                        metrics,
                        theme,
                        tool_output_open,
                        &permission_feedback,
                        slash_picker_index,
                        mention_picker_index,
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
        let total = model.combined_len() as i64;
        let target_scroll = offset.saturating_sub(viewport.viewport_height as i64 / 2);
        viewport.scroll_offset = target_scroll.clamp(0, (total - 1).max(0));
        viewport.auto_follow = viewport.scroll_offset == 0;
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
    let backend = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;
    term.clear()?;
    Ok(term)
}

fn teardown_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let _ = disable_raw_mode();
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
