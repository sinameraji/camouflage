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
    let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
    let mut viewport = ViewportState::new(session_id, height.saturating_sub(4), width);
    let mut input_buf = String::new();
    let mut status: String = "idle".into();

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

    // Outbound NDJSON emitter (renderer → host). Used for UserInputSubmitted
    // and PermissionResponse when `--emit-responses` is set.
    let outbound_tx: Option<mpsc::Sender<OutgoingEvent>> = if cfg.emit_responses {
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
            // Priority order: pending permission widget → replay controls
            // (if --replay) → normal key handler.
            let action = if model.pending_permission().is_some() {
                input::handle_key_permission(key)
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
                        let ev = Event {
                            id: Uuid::new_v4(),
                            session_id,
                            seq: seq_counter.fetch_add(1, Ordering::Relaxed),
                            timestamp_ms: now_ms(),
                            schema_version: SCHEMA_VERSION,
                            event_type: EventType::PermissionResponse,
                            payload: serde_json::json!({
                                "request_id": request_id,
                                "choice": choice_str,
                            }),
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
                model.apply(&ev);
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
                            model.apply(&ev2);
                            drained += 1;
                        }
                        Err(_) => break,
                    }
                }
            }
            _ = ticker.tick() => {
                frame_counter = frame_counter.wrapping_add(1);
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
                    draw::render(&mut terminal, &model, &viewport, &input_buf, &status, frame_counter)?;
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
        prev(info);
    }));
}
